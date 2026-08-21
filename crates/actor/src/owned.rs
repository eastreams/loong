use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tokio_util::{sync::WaitForCancellationFutureOwned, task::TaskTracker};

use crate::{
    Actor,
    mailbox::{ActorInner, Mode},
};

/// Tracks spawned Tokio tasks for borrow-free replies.
///
/// Completed tasks leave the tracker immediately.
/// Closing enables its shutdown barrier.
/// It does not reject later spawns.
/// Dispatch must therefore end before closing.
pub(crate) struct OwnedTasks<A: Actor> {
    actor: Arc<ActorInner<A>>,
    tasks: TaskTracker,
}

impl<A: Actor> OwnedTasks<A> {
    /// Creates an open tracker for one actor.
    pub(crate) fn new(actor: Arc<ActorInner<A>>) -> Self {
        Self {
            actor,
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
            cancellation: self.actor.control.owned_cancellation().cancelled_owned(),
            actor: Arc::clone(&self.actor),
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
    struct OwnedTask<A: Actor, F> {
        #[pin]
        state: OwnedTaskState<F>,
        #[pin]
        cancellation: WaitForCancellationFutureOwned,
        actor: Arc<ActorInner<A>>,
    }

    impl<A: Actor, F> PinnedDrop for OwnedTask<A, F> {
        fn drop(this: Pin<&mut Self>) {
            this.drop_future();
        }
    }
}

impl<A: Actor, F> OwnedTask<A, F> {
    /// Replaces the pinned future before its destructor can unwind.
    fn drop_future(mut self: Pin<&mut Self>) {
        let this = self.as_mut().project();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = this.state.project_replace(OwnedTaskState::Done);
        }));
        if let Err(payload) = result {
            this.actor.control.contain_panic(payload);
        }
    }
}

impl<A, F> Future for OwnedTask<A, F>
where
    A: Actor,
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
                this.actor.control.mode(),
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
                self.as_mut().project().actor.control.contain_panic(payload);
                self.as_mut().drop_future();
                Poll::Ready(())
            }
        }
    }
}

#[cfg(test)]
mod tests;
