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

struct TestActor;

#[crate::actor(mailbox = 1)]
impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut crate::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn test_actor_inner() -> Arc<ActorInner<TestActor>> {
    let options = <TestActor as crate::ActorConfig>::Options::default();
    ActorInner::open(&options).0
}

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
    let actor = test_actor_inner();
    let polls = Arc::new(AtomicUsize::new(0));
    let owned = OwnedTasks::new(Arc::clone(&actor));

    owned.spawn(PollCounter(Arc::clone(&polls)));
    actor.control.request(crate::Shutdown::Kill);
    owned.close();
    owned.wait().await;

    assert_eq!(polls.load(Ordering::SeqCst), 0);
}

// Poll, payload Drop, future Drop, and lifecycle wake all panic.
// Containment must prevent them from combining into process abort.
#[tokio::test]
async fn owned_task_contains_cascading_panics() {
    let actor = test_actor_inner();
    let mut mode = actor.control.subscribe_mode();
    let mut changed = Box::pin(mode.changed());
    let waker = Waker::from(Arc::new(PanicWake));
    let mut task = Context::from_waker(&waker);
    assert!(changed.as_mut().poll(&mut task).is_pending());

    let payload_dropped = Arc::new(AtomicBool::new(false));
    let owned = OwnedTasks::new(Arc::clone(&actor));
    owned.spawn(CascadingPanicFuture {
        payload_dropped: Arc::clone(&payload_dropped),
    });
    owned.close();
    owned.wait().await;

    assert!(payload_dropped.load(Ordering::SeqCst));
    assert_eq!(actor.control.mode(), Mode::Failing);
}
