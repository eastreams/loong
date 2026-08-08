use std::{
    future::Future,
    marker::PhantomPinned,
    num::NonZeroUsize,
    panic,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use tokio::sync::oneshot;

use crate::{
    Actor, ActorConfig, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, ExitStatus,
    IntoActorFuture, Message, MessageConfig, Shutdown, ShutdownStatus, SubtreeStatus, SyncHandler,
    actor::HasMailbox,
    mailbox::{ActorInbox, ActorInner, Control, Envelope, Mode},
    owned::OwnedTasks,
    scheduling::{
        ActorScheduler, InterleavedLane, InterleavedProfile, InterleavedScheduler, SchedulerTurn,
        Seal,
    },
    supervision::runtime::{RuntimeChildren, tests::ChildrenFixture},
    transport::MessageSender,
};

use super::{
    ActorTask, ActorWorkGuard, ActorWorkState, DiscardOutcome, DrainTurn, ExitGuard, ScopeState,
    TEARDOWN_DROP_BUDGET, Work, actor_turn, await_actor_work, close_and_discard, drain_turn,
    graceful_finish, handle_child_exit, kill_actor, run_actor,
};

mod actor_turn;
mod child_exit;
mod inbox;
mod shutdown;
mod work;

struct TestActor;

#[crate::actor(
    mailbox = dynamic,
    mailbox_budget = 3,
    interleaved = dynamic,
    children = 1
)]
impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

/// Opens the dynamic test policy with one explicit capacity.
fn test_actor_inner(capacity: usize) -> (Arc<ActorInner<TestActor>>, ActorInbox<TestActor>) {
    let capacity = NonZeroUsize::new(capacity).expect("test mailbox capacity is nonzero");
    let options = <TestActor as ActorConfig>::Options::default().with_mailbox_capacity(capacity);
    let (inner, inbox, _scheduler) = ActorInner::<TestActor>::open(&options);
    (inner, inbox)
}

/// Enqueues one fixture through the same admission boundary as production.
fn enqueue_test_envelope<A: HasMailbox>(
    inner: &ActorInner<A>,
    envelope: impl Envelope<A> + 'static,
) {
    let reservation = inner
        .sender
        .try_reserve()
        .expect("the test inbox has capacity");
    assert!(inner.admit(reservation, Box::new(envelope)).is_ok());
}

struct UnboundedTestActor;

#[crate::actor(mailbox = unbounded)]
impl Actor for UnboundedTestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn scope_state<A: Actor>(inner: &Arc<ActorInner<A>>) -> ScopeState<A> {
    let options = <A as ActorConfig>::Options::default();
    ScopeState {
        actor_ref: ActorRef::new(Arc::clone(inner)),
        children: A::open_children(&options),
    }
}

struct CascadingPanicPayload(Arc<AtomicBool>);

impl Drop for CascadingPanicPayload {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional panic payload drop panic");
    }
}

struct ActorFrameDropProbe {
    status: ExitStatus,
    dropped: Arc<AtomicBool>,
}

struct AbortedFrameDropProbe {
    inner: Arc<ActorInner<TestActor>>,
    saw_aborting: Arc<AtomicBool>,
}

impl Future for AbortedFrameDropProbe {
    type Output = ExitStatus;

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for AbortedFrameDropProbe {
    fn drop(&mut self) {
        self.saw_aborting.store(
            self.inner.control.mode() == Mode::Aborting,
            Ordering::SeqCst,
        );
        panic!("intentional aborted frame drop panic");
    }
}

impl Future for ActorFrameDropProbe {
    type Output = ExitStatus;

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.status)
    }
}

impl Drop for ActorFrameDropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        panic!("intentional actor frame drop panic");
    }
}

struct ActorWorkDropProbe<'a> {
    kill: Option<&'a Control>,
    panic_on_poll: bool,
    polled: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    _pin: PhantomPinned,
}

impl Future for ActorWorkDropProbe<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_ref().get_ref();
        this.polled.store(true, Ordering::SeqCst);
        assert!(!this.panic_on_poll, "intentional actor work poll panic");
        if let Some(control) = &this.kill {
            control.request(Shutdown::Kill);
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

impl Drop for ActorWorkDropProbe<'_> {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        panic!("intentional actor work drop panic");
    }
}

struct ActorWorkOutputDropProbe {
    dropped: Arc<AtomicBool>,
    panic_on_drop: bool,
}

impl Drop for ActorWorkOutputDropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        assert!(!self.panic_on_drop, "intentional output drop panic");
    }
}

struct ReadyActorWorkFrame {
    output: Option<ActorWorkOutputDropProbe>,
    dropped: Arc<AtomicBool>,
    panic_on_drop: bool,
}

impl Future for ReadyActorWorkFrame {
    type Output = ActorWorkOutputDropProbe;

    fn poll(mut self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(
            self.output
                .take()
                .expect("a ready frame produces its output once"),
        )
    }
}

impl Drop for ReadyActorWorkFrame {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        assert!(!self.panic_on_drop, "intentional ready frame drop panic");
    }
}

struct CountChildExit(Arc<AtomicUsize>);

#[crate::actor(children = 1)]
impl Actor for CountChildExit {
    type SpawnArgs = Arc<AtomicUsize>;

    async fn init(observed: Arc<AtomicUsize>, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(observed)
    }

    async fn on_child_exit(&mut self, _event: ChildExit, _scope: &mut ActorScope<'_, Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct AbortChildParent {
    child_exit: Option<oneshot::Sender<ExitStatus>>,
}

#[crate::actor(mailbox = 1, children = 1)]
impl Actor for AbortChildParent {
    type SpawnArgs = oneshot::Sender<ExitStatus>;

    async fn init(
        child_exit: oneshot::Sender<ExitStatus>,
        scope: &mut ActorScope<'_, Self>,
    ) -> Self {
        let (child, _inbox) = test_actor_inner(1);
        let child_ref = ActorRef::new(Arc::clone(&child));
        let child_id = scope.state.children.insert_ref(&child_ref);
        child.control.begin_abort();
        let status = child.control.finish(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        ));
        scope
            .state
            .children
            .publish(ChildExit::new(child_id, status));

        Self {
            child_exit: Some(child_exit),
        }
    }

    async fn on_child_exit(&mut self, event: ChildExit, _scope: &mut ActorScope<'_, Self>) {
        if let Some(child_exit) = self.child_exit.take() {
            let _ = child_exit.send(event.status());
        }
    }
}

#[derive(Message)]
struct Ping;

impl SyncHandler<Ping> for AbortChildParent {
    fn handle(&mut self, _message: Ping, _scope: &mut ActorScope<Self>) {}
}

struct ControlledChildExit {
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    completed: Option<oneshot::Sender<()>>,
}

#[crate::actor(children = 1)]
impl Actor for ControlledChildExit {
    type SpawnArgs = Self;

    async fn init(actor: Self, _scope: &mut ActorScope<'_, Self>) -> Self {
        actor
    }

    async fn on_child_exit(&mut self, _event: ChildExit, _scope: &mut ActorScope<'_, Self>) {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.release.take() {
            let _ = release.await;
        }
        if let Some(completed) = self.completed.take() {
            let _ = completed.send(());
        }
    }
}

struct CountEnvelope(Arc<AtomicUsize>);

impl<A: Actor> Envelope<A> for CountEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut A,
        _scope: &mut ActorScope<A>,
        _owned: &OwnedTasks<A>,
        _scheduler: &mut ActorScheduler<A>,
        _inner: &Arc<ActorInner<A>>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

struct PanicEnvelope;

impl Envelope<TestActor> for PanicEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        _scheduler: &mut ActorScheduler<TestActor>,
        _inner: &Arc<ActorInner<TestActor>>,
    ) {
        panic!("intentional dispatch panic");
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

struct ChildKillDropProbe {
    child: Arc<ActorInner<TestActor>>,
    observed_kill: Arc<AtomicBool>,
}

impl Future for ChildKillDropProbe {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for ChildKillDropProbe {
    fn drop(&mut self) {
        let mode = self.child.control.mode();
        self.observed_kill.store(
            mode == Mode::Killing
                || matches!(
                    mode,
                    Mode::Exited(status) if status.reason() == ExitReason::Killed
                ),
            Ordering::SeqCst,
        );
    }
}

impl Envelope<TestActor> for ChildKillDropProbe {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        _scheduler: &mut ActorScheduler<TestActor>,
        _inner: &Arc<ActorInner<TestActor>>,
    ) {
        unreachable!("Kill discards queued envelopes")
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

enum TeardownEnvelope {
    Noop,
    Panic(Arc<AtomicBool>),
    RequestKill(Weak<ActorInner<TestActor>>),
    MarkDropped(Arc<AtomicBool>),
    TrackDrop {
        dropped: Arc<AtomicBool>,
        dropped_while_unwinding: Arc<AtomicBool>,
    },
}

impl Drop for TeardownEnvelope {
    fn drop(&mut self) {
        match self {
            Self::Noop => {}
            Self::Panic(observed) => {
                observed.store(true, Ordering::SeqCst);
                panic!("intentional envelope drop panic");
            }
            Self::RequestKill(control) => {
                control
                    .upgrade()
                    .expect("the teardown fixture retains its actor")
                    .control
                    .request(Shutdown::Kill);
            }
            Self::MarkDropped(observed) => observed.store(true, Ordering::SeqCst),
            Self::TrackDrop {
                dropped,
                dropped_while_unwinding,
            } => {
                dropped.store(true, Ordering::SeqCst);
                dropped_while_unwinding.store(std::thread::panicking(), Ordering::SeqCst);
            }
        }
    }
}

impl<A: Actor> Envelope<A> for TeardownEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut A,
        _scope: &mut ActorScope<A>,
        _owned: &OwnedTasks<A>,
        _scheduler: &mut ActorScheduler<A>,
        _inner: &Arc<ActorInner<A>>,
    ) {
        unreachable!("teardown discards queued envelopes")
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}
