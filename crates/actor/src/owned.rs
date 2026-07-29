use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tokio_util::{sync::WaitForCancellationFutureOwned, task::TaskTracker};

use crate::mailbox::{Control, Mode};

/// Tracks spawned Tokio tasks for borrow-free replies.
///
/// Completed tasks leave the tracker immediately.
/// Closing enables its shutdown barrier.
/// It does not reject later spawns.
/// Dispatch must therefore end before closing.
pub(crate) struct OwnedTasks {
    control: Arc<Control>,
    tasks: TaskTracker,
}

impl OwnedTasks {
    /// Creates an open tracker for one actor.
    pub(crate) fn new(control: Arc<Control>) -> Self {
        Self {
            control,
            tasks: TaskTracker::new(),
        }
    }

    /// Spawns and registers one owned reply task.
    pub(crate) fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = OwnedTask {
            state: OwnedTaskState::Running { future },
            cancellation: self.control.owned_cancellation().cancelled_owned(),
            control: Arc::clone(&self.control),
        };
        // The tracker supplies one barrier; no task-specific handle escapes.
        drop(self.tasks.spawn(task));
    }

    /// Closes after dispatch can no longer spawn owned work.
    pub(crate) fn close(&self) {
        self.tasks.close();
    }

    /// Waits until the closed tracker becomes empty.
    /// Every tracked future is fully destroyed first.
    pub(crate) async fn wait(&self) {
        self.tasks.wait().await;
    }
}

pin_project! {
    #[project = OwnedTaskStateProj]
    #[project_replace = OwnedTaskStateProjReplace]
    enum OwnedTaskState<F> {
        Running {
            #[pin]
            future: F,
        },
        Done,
    }
}

pin_project! {
    /// Contains polling and destruction for one borrow-free reply.
    struct OwnedTask<F> {
        #[pin]
        state: OwnedTaskState<F>,
        #[pin]
        cancellation: WaitForCancellationFutureOwned,
        control: Arc<Control>,
    }

    impl<F> PinnedDrop for OwnedTask<F> {
        fn drop(this: Pin<&mut Self>) {
            this.drop_future();
        }
    }
}

impl<F> OwnedTask<F> {
    /// Replaces the pinned future before its destructor can unwind.
    fn drop_future(mut self: Pin<&mut Self>) {
        let this = self.as_mut().project();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = this.state.project_replace(OwnedTaskState::Done);
        }));
        if let Err(payload) = result {
            this.control.contain_panic(payload);
        }
    }
}

impl<F> Future for OwnedTask<F>
where
    F: Future<Output = ()>,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        let must_drop = {
            let this = self.as_mut().project();
            // The token only registers this task's wake.
            // Mode remains the sole cancellation decision.
            let _ = this.cancellation.poll(task);
            !matches!(
                this.control.mode(),
                Mode::Running | Mode::Draining | Mode::Stopping
            )
        };
        if must_drop {
            self.as_mut().drop_future();
            return Poll::Ready(());
        }

        let result = {
            let this = self.as_mut().project();
            panic::catch_unwind(AssertUnwindSafe(|| match this.state.project() {
                OwnedTaskStateProj::Running { future } => future.poll(task),
                OwnedTaskStateProj::Done => panic!("a completed owned task cannot be polled"),
            }))
        };
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(())) => {
                self.as_mut().drop_future();
                Poll::Ready(())
            }
            Err(payload) => {
                self.as_mut().project().control.contain_panic(payload);
                self.as_mut().drop_future();
                Poll::Ready(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        panic,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
    };

    use super::*;

    struct PollCounter(Arc<AtomicUsize>);

    impl Future for PollCounter {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Poll::Pending
        }
    }

    struct PanicPayload(Arc<AtomicBool>);

    impl Drop for PanicPayload {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            panic!("intentional panic payload drop panic");
        }
    }

    struct CascadingPanicFuture {
        payload_dropped: Arc<AtomicBool>,
    }

    impl Future for CascadingPanicFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
            panic::panic_any(PanicPayload(Arc::clone(&self.payload_dropped)))
        }
    }

    impl Drop for CascadingPanicFuture {
        fn drop(&mut self) {
            panic!("intentional owned future drop panic");
        }
    }

    struct PanicWake;

    impl Wake for PanicWake {
        fn wake(self: Arc<Self>) {
            panic!("intentional lifecycle waker panic");
        }

        fn wake_by_ref(self: &Arc<Self>) {
            panic!("intentional lifecycle waker panic");
        }
    }

    // Kill may commit before Tokio first polls a spawned task.
    // The task must observe that cutoff before entering user code.
    #[tokio::test(flavor = "current_thread")]
    async fn owned_task_does_not_poll_after_kill() {
        let control = Control::new();
        let polls = Arc::new(AtomicUsize::new(0));
        let owned = OwnedTasks::new(Arc::clone(&control));

        owned.spawn(PollCounter(Arc::clone(&polls)));
        control.request(crate::Shutdown::Kill);
        owned.close();
        owned.wait().await;

        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }

    // Poll, payload Drop, future Drop, and lifecycle wake all panic.
    // Containment must prevent them from combining into process abort.
    #[tokio::test]
    async fn owned_task_contains_cascading_panics() {
        let control = Control::new();
        let mut mode = control.subscribe_mode();
        let mut changed = Box::pin(mode.changed());
        let waker = Waker::from(Arc::new(PanicWake));
        let mut task = Context::from_waker(&waker);
        assert!(changed.as_mut().poll(&mut task).is_pending());

        let payload_dropped = Arc::new(AtomicBool::new(false));
        let owned = OwnedTasks::new(Arc::clone(&control));
        owned.spawn(CascadingPanicFuture {
            payload_dropped: Arc::clone(&payload_dropped),
        });
        owned.close();
        owned.wait().await;

        assert!(payload_dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), Mode::Failing);
    }
}
