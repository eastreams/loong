use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use tokio::sync::oneshot;

use crate::{
    Actor, ActorScope, CallError, ExitReason, Handler, Message, ReplyExt, Shutdown, ShutdownStatus,
    owned::OwnedTasks, scheduler::ReplyScheduler,
};

use super::super::{ActorInner, CallEnvelope, Envelope, Mode};
use super::{PanicWake, WakeCounter};

struct TestActor;

impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct NoopEnvelope;

impl Envelope<TestActor> for NoopEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        _scheduler: &mut ReplyScheduler<TestActor>,
        _inner: &Arc<ActorInner<TestActor>>,
    ) {
    }
}

struct RecoverMessage(Arc<AtomicUsize>);

impl Drop for RecoverMessage {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Message for RecoverMessage {
    type Reply = ();
}

impl Handler<RecoverMessage> for TestActor {
    fn handle(
        &mut self,
        _message: RecoverMessage,
        _scope: &mut ActorScope<Self>,
    ) -> impl crate::IntoReply<Self, RecoverMessage> + use<> {
        ().ready()
    }
}

// A capacity reservation is not admission. Once Drain wins the lifecycle
// transaction, the reserved slot must be returned without entering inbox.
#[tokio::test]
async fn shutdown_wins_over_an_acquired_but_uncommitted_permit() {
    let (inner, inbox) = ActorInner::<TestActor>::channel(1);
    let permit = inner
        .sender
        .reserve()
        .await
        .expect("the test mailbox is open");

    assert_eq!(
        inner.control.request(Shutdown::Drain),
        ShutdownStatus::Requested
    );
    let committed = inner.admit(permit, Box::new(NoopEnvelope));

    let Err((permit, envelope)) = committed else {
        panic!("shutdown must reject the uncommitted envelope");
    };
    drop(permit);
    drop(envelope);
    assert!(inbox.is_empty());
}

#[tokio::test]
async fn committed_send_is_part_of_the_fixed_drain_queue() {
    // Observe the inbox directly to isolate the admission/Drain ordering:
    // once admission wins the shared gate, Drain must retain that envelope.
    let (inner, mut inbox) = ActorInner::<TestActor>::channel(1);
    let permit = inner
        .sender
        .reserve()
        .await
        .expect("the test mailbox is open");

    let committed = inner.admit(permit, Box::new(NoopEnvelope));
    let Ok(()) = committed else {
        panic!("the commit must win admission");
    };
    assert_eq!(
        inner.control.request(Shutdown::Drain),
        ShutdownStatus::Requested
    );

    assert!(inbox.try_discard());
    assert!(inbox.is_empty());
}

// The typed rejected path must recover the concrete call message rather
// than dropping CallEnvelope and publishing a fabricated queued failure.
#[tokio::test]
async fn rejected_call_admission_recovers_its_message_without_a_reply() {
    let (inner, inbox) = ActorInner::<TestActor>::channel(1);
    let permit = inner
        .sender
        .reserve()
        .await
        .expect("the test mailbox is open");
    let drops = Arc::new(AtomicUsize::new(0));
    let (envelope, mut response) = CallEnvelope::new(RecoverMessage(Arc::clone(&drops)), &inner);
    assert_eq!(
        inner.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    let rejected = inner.admit(permit, Box::new(envelope));
    let Err((permit, envelope)) = rejected else {
        panic!("shutdown must reject the call envelope");
    };
    drop(permit);
    let message = (*envelope).into_message();

    assert_eq!(
        response.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(message);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(inbox.is_empty());
}

// Admission recovery notifies a still-live response receiver.
// Its Waker panic must not consume the recovered message.
// Uncommitted work must not fail the actor.
#[test]
fn call_recovery_contains_response_waker_panic() {
    let (actor, _inbox) = ActorInner::<TestActor>::channel(1);
    let message_drops = Arc::new(AtomicUsize::new(0));
    let (envelope, response) =
        CallEnvelope::new(RecoverMessage(Arc::clone(&message_drops)), &actor);
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut task = Context::from_waker(&waker);
    let mut response = Box::pin(response);
    assert!(response.as_mut().poll(&mut task).is_pending());

    let recovered = panic::catch_unwind(AssertUnwindSafe(|| envelope.into_message()));

    assert!(recovered.is_ok());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(message_drops.load(Ordering::SeqCst), 0);
    assert_eq!(actor.control.mode(), Mode::Running);
    drop(recovered.unwrap());
    assert_eq!(message_drops.load(Ordering::SeqCst), 1);
}

struct PanicDropMessage(Arc<AtomicBool>);

impl Message for PanicDropMessage {
    type Reply = ();
}

impl Drop for PanicDropMessage {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional message drop panic");
    }
}

// Queued rejection first notifies the caller, then drops its message.
// Both callbacks may panic and must remain separate containment boundaries.
#[test]
fn queued_call_drop_contains_notification_and_message_drop_panics() {
    let (actor, _inbox) = ActorInner::<TestActor>::channel(1);
    let message_dropped = Arc::new(AtomicBool::new(false));
    let (envelope, response) =
        CallEnvelope::new(PanicDropMessage(Arc::clone(&message_dropped)), &actor);
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut task = Context::from_waker(&waker);
    let mut response = Box::pin(response);
    assert!(response.as_mut().poll(&mut task).is_pending());

    let dropped = panic::catch_unwind(AssertUnwindSafe(|| drop(envelope)));

    assert!(dropped.is_ok());
    assert!(message_dropped.load(Ordering::SeqCst));
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(actor.control.mode(), Mode::Failing);
    assert!(matches!(
        response.as_mut().get_mut().try_recv(),
        Ok(Err(CallError::BeforeDispatch(ExitReason::Panicked)))
    ));
}

// Running-to-Running admission is the hot path. It must take the same write
// gate without publishing a fake lifecycle change to every closed() waiter.
#[test]
fn mailbox_admission_does_not_wake_lifecycle_observers() {
    let (inner, mut inbox) = ActorInner::<TestActor>::channel(1);
    let mut mode = inner.control.subscribe_mode();
    let mut changed = Box::pin(mode.changed());
    let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);
    let permit = inner
        .sender
        .try_reserve()
        .expect("the test mailbox has capacity");

    assert!(matches!(changed.as_mut().poll(&mut task), Poll::Pending));
    let admitted = inner.admit(permit, Box::new(NoopEnvelope));
    let Ok(()) = admitted else {
        panic!("Running must admit the envelope");
    };
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert!(matches!(changed.as_mut().poll(&mut task), Poll::Pending));

    drop(changed);
    assert!(
        !mode
            .has_changed()
            .expect("the control still owns its sender")
    );
    assert!(inbox.try_discard());
}
