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
    Actor, ActorConfig, ActorScope, CallError, ExitReason, Handler, Message, ReplyExt, Shutdown,
    ShutdownStatus, owned::OwnedTasks, scheduler::ReplyScheduler, transport::MessageSender,
};

use super::super::{ActorInbox, ActorInner, CallEnvelope, Control, Envelope, Mode};
use super::{PanicWake, WakeCounter};

struct TestActor;

#[crate::actor(mailbox = 1)]
impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct UnboundedTestActor;

#[crate::actor(mailbox = unbounded)]
impl Actor for UnboundedTestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct NoopEnvelope;

impl<A: Actor> Envelope<A> for NoopEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut A,
        _scope: &mut ActorScope<A>,
        _owned: &OwnedTasks<A>,
        _scheduler: &mut ReplyScheduler<A>,
        _inner: &Arc<ActorInner<A>>,
    ) {
    }

    fn discard(self: Box<Self>, _control: &Control) {}
}

struct PanicDropEnvelope(Arc<AtomicBool>);

impl Drop for PanicDropEnvelope {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional envelope drop panic");
    }
}

impl Envelope<UnboundedTestActor> for PanicDropEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut UnboundedTestActor,
        _scope: &mut ActorScope<UnboundedTestActor>,
        _owned: &OwnedTasks<UnboundedTestActor>,
        _scheduler: &mut ReplyScheduler<UnboundedTestActor>,
        _inner: &Arc<ActorInner<UnboundedTestActor>>,
    ) {
        unreachable!("the broken backend cannot dispatch its envelope")
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

fn open<A: Actor>() -> (Arc<ActorInner<A>>, ActorInbox<A>) {
    let options = <A as ActorConfig>::Options::default();
    ActorInner::open(&options)
}

#[derive(Message)]
struct RecoverMessage(Arc<AtomicUsize>);

impl Drop for RecoverMessage {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
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
#[test]
fn shutdown_wins_over_an_acquired_but_uncommitted_permit() {
    let (inner, inbox) = open::<TestActor>();
    let permit = inner
        .sender
        .try_reserve()
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

#[test]
fn committed_send_is_part_of_the_fixed_drain_queue() {
    // Observe the inbox directly to isolate the admission/Drain ordering:
    // once admission wins the shared gate, Drain must retain that envelope.
    let (inner, mut inbox) = open::<TestActor>();
    let permit = inner
        .sender
        .try_reserve()
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

#[test]
fn unbounded_reservation_still_uses_lifecycle_admission() {
    let (inner, inbox) = open::<UnboundedTestActor>();
    let reservation = inner
        .sender
        .try_reserve()
        .expect("the unbounded inbox is open");
    assert_eq!(
        inner.control.request(Shutdown::Drain),
        ShutdownStatus::Requested
    );

    let Err((_reservation, envelope)) = inner.admit(reservation, Box::new(NoopEnvelope)) else {
        panic!("Drain must reject an uncommitted unbounded envelope");
    };
    drop(envelope);
    assert!(inbox.is_empty());
}

// Running requires every backend receiver to remain open. A violation must
// contain the erased value, fail the actor, and remain visible as a panic.
#[test]
fn transport_closure_during_running_is_a_contained_invariant_failure() {
    let (inner, mut inbox) = open::<UnboundedTestActor>();
    let reservation = inner
        .sender
        .try_reserve()
        .expect("the unbounded inbox is initially open");
    inbox.close();
    let dropped = Arc::new(AtomicBool::new(false));

    let failure = panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = inner.admit(
            reservation,
            Box::new(PanicDropEnvelope(Arc::clone(&dropped))),
        );
    }));

    assert!(failure.is_err());
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(inner.control.mode(), Mode::Failing);
    assert!(inbox.is_empty());
}

// The typed rejected path must recover the concrete call message rather
// than dropping CallEnvelope and publishing a fabricated queued failure.
#[test]
fn rejected_call_admission_recovers_its_message_without_a_reply() {
    let (inner, inbox) = open::<TestActor>();
    let permit = inner
        .sender
        .try_reserve()
        .expect("the test mailbox is open");
    let drops = Arc::new(AtomicUsize::new(0));
    let (envelope, mut response) = CallEnvelope::new(RecoverMessage(Arc::clone(&drops)));
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
    let (actor, _inbox) = open::<TestActor>();
    let message_drops = Arc::new(AtomicUsize::new(0));
    let (envelope, response) = CallEnvelope::new(RecoverMessage(Arc::clone(&message_drops)));
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

// A malformed transport may drop its opaque carrier.
// That violation closes the reply without touching lifecycle state.
// Conforming transports always return accepted work to the runtime.
#[test]
fn raw_call_envelope_drop_has_no_lifecycle_callback() {
    let (actor, _inbox) = open::<TestActor>();
    let message_drops = Arc::new(AtomicUsize::new(0));
    let (envelope, mut response) = CallEnvelope::new(RecoverMessage(Arc::clone(&message_drops)));

    drop(envelope);

    assert_eq!(
        response.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    );
    assert_eq!(message_drops.load(Ordering::SeqCst), 1);
    assert_eq!(actor.control.mode(), Mode::Running);
}

#[derive(Message)]
struct PanicDropMessage(Arc<AtomicBool>);

impl Drop for PanicDropMessage {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional message drop panic");
    }
}

impl Handler<PanicDropMessage> for TestActor {
    fn handle(
        &mut self,
        _message: PanicDropMessage,
        _scope: &mut ActorScope<Self>,
    ) -> impl crate::IntoReply<Self, PanicDropMessage> + use<> {
        ().ready()
    }
}

// Queued rejection first notifies the caller, then drops its message.
// Both callbacks may panic and must remain separate containment boundaries.
#[test]
fn queued_call_discard_contains_notification_and_message_drop_panics() {
    let (actor, _inbox) = open::<TestActor>();
    let message_dropped = Arc::new(AtomicBool::new(false));
    let (envelope, response) = CallEnvelope::new(PanicDropMessage(Arc::clone(&message_dropped)));
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut task = Context::from_waker(&waker);
    let mut response = Box::pin(response);
    assert!(response.as_mut().poll(&mut task).is_pending());

    let dropped = panic::catch_unwind(AssertUnwindSafe(|| {
        <CallEnvelope<PanicDropMessage> as Envelope<TestActor>>::discard(
            Box::new(envelope),
            &actor.control,
        );
    }));

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
    let (inner, mut inbox) = open::<TestActor>();
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
