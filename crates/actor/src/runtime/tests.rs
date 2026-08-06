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
    spawn,
    supervision::runtime::{RuntimeChildren, tests::ChildrenFixture},
    transport::MessageSender,
};

use super::{
    ActorTask, ActorWorkGuard, ActorWorkState, DiscardOutcome, DrainTurn, ExitGuard, ScopeState,
    TEARDOWN_DROP_BUDGET, Work, actor_turn, await_actor_work, close_and_discard, drain_turn,
    graceful_finish, handle_child_exit, kill_actor, run_actor,
};

mod actor_turn;

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

// A catch owns its panic payload after the first unwind ends.
// Payload destruction must not start a second runtime unwind.
#[tokio::test]
async fn actor_work_contains_panic_payload_destruction() {
    let control = Control::new();
    let payload_dropped = Arc::new(AtomicBool::new(false));

    let work = await_actor_work(
        async {
            panic::panic_any(CascadingPanicPayload(Arc::clone(&payload_dropped)));
        },
        &control,
    )
    .await;

    assert!(matches!(work, Work::Panicked));
    assert!(payload_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Failing);
}

// Ready Drop is checked in every graceful mode.
// Kill after entry locks hard-cutoff ordering.
// Poll panic proves polling and Drop use separate boundaries.
// The !Unpin probe rejects move-based cleanup.
#[tokio::test]
async fn actor_work_contains_future_drop_panics() {
    for (initial, kill_on_poll, panic_on_poll, expected_work, expected_mode) in [
        (None, false, false, Work::DropPanicked(()), Mode::Failing),
        (
            Some(Shutdown::Stop),
            false,
            false,
            Work::DropPanicked(()),
            Mode::Failing,
        ),
        (
            Some(Shutdown::Drain),
            false,
            false,
            Work::DropPanicked(()),
            Mode::Failing,
        ),
        (None, true, false, Work::Killed, Mode::Killing),
        (None, false, true, Work::Panicked, Mode::Failing),
    ] {
        let control = Control::new();
        if let Some(shutdown) = initial {
            assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        }
        let polled = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));

        let work = await_actor_work(
            ActorWorkDropProbe {
                kill: kill_on_poll.then_some(&control),
                panic_on_poll,
                polled: Arc::clone(&polled),
                dropped: Arc::clone(&dropped),
                _pin: PhantomPinned,
            },
            &control,
        )
        .await;

        assert_eq!(work, expected_work);
        assert!(polled.load(Ordering::SeqCst));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), expected_mode);
    }
}

// Output may leave containment only after its ready frame is retired.
// Returning first would leave user frame cleanup pending after delivery.
#[test]
fn actor_work_retires_ready_frame_before_delivering_output() {
    let control = Control::new();
    let frame_dropped = Arc::new(AtomicBool::new(false));
    let output_dropped = Arc::new(AtomicBool::new(false));
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: ReadyActorWorkFrame {
                output: Some(ActorWorkOutputDropProbe {
                    dropped: Arc::clone(&output_dropped),
                    panic_on_drop: false,
                }),
                dropped: Arc::clone(&frame_dropped),
                panic_on_drop: false,
            },
        },
        control: &control,
    });

    let Poll::Ready(Work::Complete(output)) = guarded.as_mut().poll(&mut task) else {
        panic!("ready work must deliver its output");
    };

    assert!(frame_dropped.load(Ordering::SeqCst));
    assert!(!output_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Running);
    drop(output);
    assert!(output_dropped.load(Ordering::SeqCst));
}

// A frame Drop panic invalidates its ready output.
// The lifecycle owner retains that output for ordered teardown.
#[test]
fn actor_work_retains_invalidated_ready_output_for_ordered_drop() {
    let control = Control::new();
    let frame_dropped = Arc::new(AtomicBool::new(false));
    let output_dropped = Arc::new(AtomicBool::new(false));
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: ReadyActorWorkFrame {
                output: Some(ActorWorkOutputDropProbe {
                    dropped: Arc::clone(&output_dropped),
                    panic_on_drop: true,
                }),
                dropped: Arc::clone(&frame_dropped),
                panic_on_drop: true,
            },
        },
        control: &control,
    });

    let Poll::Ready(Work::DropPanicked(output)) = guarded.as_mut().poll(&mut task) else {
        panic!("frame Drop failure must retain its ready output");
    };
    assert!(frame_dropped.load(Ordering::SeqCst));
    assert!(!output_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Failing);

    control.drop_user_value(output);
    assert!(output_dropped.load(Ordering::SeqCst));
}

// Polling after Ready is a runtime contract violation, not an actor panic.
// The invariant check must remain outside the user future panic boundary.
#[test]
fn completed_actor_work_repoll_exposes_the_runtime_bug() {
    let control = Control::new();
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: std::future::ready(()),
        },
        control: &control,
    });
    assert_eq!(
        guarded.as_mut().poll(&mut task),
        Poll::Ready(Work::Complete(()))
    );

    let repoll = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let _ = guarded.as_mut().poll(&mut task);
    }));

    assert!(repoll.is_err());
    assert_eq!(control.mode(), Mode::Running);
}

// Final frame Drop occurs before terminal publication.
// Its panic loses only when a committed Kill already won.
#[test]
fn actor_task_contains_final_frame_drop_panic() {
    for (shutdown, expected_reason) in [
        (None, ExitReason::Panicked),
        (Some(Shutdown::Kill), ExitReason::Killed),
    ] {
        let (inner, _inbox) = test_actor_inner(1);
        let control = &inner.control;
        if let Some(shutdown) = shutdown {
            assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut actor_task = Box::pin(ActorTask::new(
            Box::pin(ActorFrameDropProbe {
                status: ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
                dropped: Arc::clone(&dropped),
            }),
            ExitGuard::new(Arc::clone(&inner), None),
        ));
        let mut task = Context::from_waker(Waker::noop());
        let expected = ExitStatus::new(expected_reason, SubtreeStatus::Terminated);

        assert_eq!(actor_task.as_mut().poll(&mut task), Poll::Ready(expected));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), Mode::Exited(expected));
    }
}

// Executor teardown commits Aborting before dropping the frame.
// A contained Drop panic cannot escape or strengthen Aborted.
#[test]
fn actor_task_contains_aborted_frame_drop_panic() {
    let (inner, _inbox) = test_actor_inner(1);
    let control = &inner.control;
    let saw_aborting = Arc::new(AtomicBool::new(false));
    let actor_task = ActorTask::new(
        Box::pin(AbortedFrameDropProbe {
            inner: Arc::clone(&inner),
            saw_aborting: Arc::clone(&saw_aborting),
        }),
        ExitGuard::new(Arc::clone(&inner), None),
    );

    drop(actor_task);

    assert!(saw_aborting.load(Ordering::SeqCst));
    assert_eq!(
        control.mode(),
        Mode::Exited(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        ))
    );
}

// An unfinished guard means terminal publication never completed.
// Its fallback must override any tentative hard mode.
#[test]
fn unfinished_exit_guard_overrides_tentative_hard_mode() {
    let expected = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);

    let (killing, _inbox) = test_actor_inner(1);
    assert_eq!(
        killing.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    drop(ExitGuard::new(Arc::clone(&killing), None));
    assert_eq!(killing.control.exit_status(), Some(expected));

    let (failing, _inbox) = test_actor_inner(1);
    failing.control.begin_failure();
    drop(ExitGuard::new(Arc::clone(&failing), None));
    assert_eq!(failing.control.exit_status(), Some(expected));
}

// An aborted child reports uncertainty without stopping a running parent.
// The parent must still dispatch messages before a later Stop.
#[tokio::test]
async fn aborted_child_does_not_stop_running_parent() {
    let (child_exit_tx, child_exit_rx) = oneshot::channel();
    let owner = spawn::<AbortChildParent>(child_exit_tx);
    let actor = owner.actor_ref();

    let child_status = tokio::time::timeout(Duration::from_secs(1), child_exit_rx)
        .await
        .expect("the running parent must consume the child event")
        .expect("the child hook must complete");
    assert_eq!(child_status.reason(), ExitReason::Aborted);
    assert_eq!(child_status.subtree(), SubtreeStatus::Unconfirmed);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), actor.call(Ping))
            .await
            .expect("the parent must still dispatch mailbox work"),
        Ok(())
    );

    let status = tokio::time::timeout(Duration::from_secs(1), owner.shutdown(Shutdown::Stop))
        .await
        .expect("parent Stop must finish");
    assert_eq!(
        status,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed)
    );
}

// A descendant abort weakens only the parent's subtree guarantee.
// The parent's own shutdown or panic reason must remain intact.
#[tokio::test]
async fn aborted_descendant_only_weakens_parent_subtree_status() {
    enum ParentExit {
        Shutdown(Shutdown),
        Panic,
    }

    for (exit, expected_reason) in [
        (ParentExit::Shutdown(Shutdown::Stop), ExitReason::Stopped),
        (ParentExit::Shutdown(Shutdown::Drain), ExitReason::Drained),
        (ParentExit::Shutdown(Shutdown::Kill), ExitReason::Killed),
        (ParentExit::Panic, ExitReason::Panicked),
    ] {
        let (child, _child_inbox) = test_actor_inner(1);
        child.control.begin_abort();
        child.control.finish(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        ));
        let (inner, inbox) = test_actor_inner(1);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        scope.children.insert_ref(&child_ref);

        match exit {
            ParentExit::Shutdown(shutdown) => {
                assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
            }
            ParentExit::Panic => enqueue_test_envelope(&inner, PanicEnvelope),
        }

        let options =
            <TestActor as ActorConfig>::Options::default().with_max_in_flight(NonZeroUsize::MIN);
        let (_, _, scheduler) = TestActor::open(&options);
        let task = ActorTask::new(
            Box::pin(run_actor::<TestActor>((), scope, inbox, scheduler)),
            ExitGuard::new(Arc::clone(&inner), None),
        );
        let status = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("parent teardown must finish");
        let expected = ExitStatus::new(expected_reason, SubtreeStatus::Unconfirmed);

        assert_eq!(status, expected);
        assert_eq!(control.exit_status(), Some(expected));
    }
}

// Dequeue observes completion but cannot release ownership.
// Reaping happens only when the event is handled.
#[tokio::test]
async fn dequeued_child_exit_keeps_registration_until_handled() {
    let observed = Arc::new(AtomicUsize::new(0));
    let (child, _child_inbox) = test_actor_inner(1);
    let status = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated);
    child.control.finish(status);

    let options = <CountChildExit as ActorConfig>::Options::default();
    let (inner, _inbox, _scheduler) = ActorInner::<CountChildExit>::open(&options);
    let control = &inner.control;
    let mut state = scope_state(&inner);
    let child_ref = ActorRef::new(Arc::clone(&child));
    let child_id = state.children.insert_ref(&child_ref);
    state.children.publish(ChildExit::new(child_id, status));
    let event = std::future::poll_fn(|task| state.poll_child_exit(task)).await;
    assert_eq!(state.children.len(), 1);

    let mut actor = CountChildExit(Arc::clone(&observed));
    assert!(matches!(
        handle_child_exit(&mut actor, &mut state, event, control).await,
        Work::Complete(())
    ));
    assert_eq!(state.children.len(), 0);
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}

// This recreates the original race window after actor_turn has dequeued a
// valid event. A graceful cutoff that commits in that window must retire the
// child without entering user code, for both graceful modes.
// A weak grandchild status must survive a normal direct-child reason.
#[tokio::test]
async fn graceful_cutoff_absorbs_a_dequeued_child_exit() {
    for shutdown in [Shutdown::Stop, Shutdown::Drain] {
        let strong = match shutdown {
            Shutdown::Stop => ExitReason::Stopped,
            Shutdown::Drain => ExitReason::Drained,
            Shutdown::Kill => unreachable!("the test uses graceful modes"),
        };
        let observed = Arc::new(AtomicUsize::new(0));
        let (child, _child_inbox) = test_actor_inner(1);

        let options = <CountChildExit as ActorConfig>::Options::default();
        let (inner, _inbox, _scheduler) = ActorInner::<CountChildExit>::open(&options);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        let child_id = scope.children.insert_ref(&child_ref);
        let mut actor = CountChildExit(Arc::clone(&observed));
        assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        assert!(matches!(
            handle_child_exit(
                &mut actor,
                &mut scope,
                ChildExit::new(
                    child_id,
                    ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed),
                ),
                control,
            )
            .await,
            Work::Complete(())
        ));
        assert_eq!(observed.load(Ordering::SeqCst), 0);
        assert_eq!(scope.children.len(), 0);
        assert_eq!(
            scope.children.terminal_status(strong),
            ExitStatus::new(strong, SubtreeStatus::Unconfirmed)
        );
    }
}

// Once hook entry wins the lifecycle gate, Stop and Drain must wait for that
// serial hook rather than cancelling it or publishing graceful completion
// around it. Kill cancellation is covered separately by lifecycle hook tests.
#[tokio::test]
async fn admitted_child_exit_hook_finishes_across_graceful_cutoff() {
    for shutdown in [Shutdown::Stop, Shutdown::Drain] {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let (child, _child_inbox) = test_actor_inner(1);

        let options = <ControlledChildExit as ActorConfig>::Options::default();
        let (inner, _inbox, _scheduler) = ActorInner::<ControlledChildExit>::open(&options);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        let child_id = scope.children.insert_ref(&child_ref);
        let mut actor = ControlledChildExit {
            entered: Some(entered_tx),
            release: Some(release_rx),
            completed: Some(completed_tx),
        };
        let controller = ActorRef::new(Arc::clone(&inner));

        let (work, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                handle_child_exit(
                    &mut actor,
                    &mut scope,
                    ChildExit::new(
                        child_id,
                        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
                    ),
                    control,
                ),
                async move {
                    entered_rx.await.unwrap();
                    assert_eq!(
                        controller.request_shutdown(shutdown),
                        ShutdownStatus::Requested
                    );
                    release_tx.send(()).unwrap();
                }
            )
        })
        .await
        .expect("an admitted hook must remain live across graceful cutoff");

        assert!(matches!(work, Work::Complete(())));
        completed_rx.await.unwrap();
        assert_eq!(scope.children.len(), 0);
    }
}

// Reserving capacity is not admission. Drain must finish from the stable queue
// snapshot even if an internal raw permit remains alive and keeps mpsc from
// reporting channel termination.
#[tokio::test]
async fn unadmitted_mailbox_permit_does_not_extend_drain() {
    let (inner, inbox) = test_actor_inner(1);
    let permit = inner
        .sender
        .reserve_owned()
        .await
        .expect("the test mailbox is open");
    let control = &inner.control;
    let scope = scope_state(&inner);
    assert_eq!(control.request(Shutdown::Drain), ShutdownStatus::Requested);

    let options = <TestActor as ActorConfig>::Options::default()
        .with_max_in_flight(NonZeroUsize::new(1).unwrap());
    let (_, _, scheduler) = TestActor::open(&options);
    let status = tokio::time::timeout(
        Duration::from_secs(1),
        run_actor::<TestActor>((), scope, inbox, scheduler),
    )
    .await
    .expect("an unadmitted capacity permit must not hold Drain open");

    assert_eq!(
        status,
        ExitStatus::new(ExitReason::Drained, SubtreeStatus::Terminated)
    );
    drop(permit);
}

#[tokio::test]
async fn committed_kill_prevents_a_graceful_child_request() {
    // The child mode distinguishes biased Kill observation from an incorrect
    // first poll of the guarded future: request_all would synchronously commit
    // Stop before graceful_finish could return Work::Killed.
    let (child, _child_inbox) = test_actor_inner(1);

    let (inner, _inbox) = test_actor_inner(1);
    let control = &inner.control;
    let mut scope = scope_state(&inner);
    let child_ref = ActorRef::new(Arc::clone(&child));
    scope.children.insert_ref(&child_ref);
    let mut actor = TestActor;

    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);
    assert!(matches!(
        graceful_finish(
            &mut actor,
            &mut scope,
            control,
            Shutdown::Stop,
            ExitReason::Stopped,
        )
        .await,
        Work::Killed
    ));
    assert_eq!(child.control.mode(), Mode::Running);
}

// A truncated reply sweep must yield before polling another ready lane.
// Otherwise ready mailbox traffic can erase the cooperative boundary.
#[test]
fn truncated_reply_sweep_yields_before_ready_mailbox() {
    // Seventeen ready replies equal the poll budget plus one.
    const REPLIES: usize = 17;

    let mailbox_dispatches = Arc::new(AtomicUsize::new(0));
    let replies_polled = Arc::new(AtomicUsize::new(0));
    let (inner, mut inbox) = test_actor_inner(1);
    enqueue_test_envelope(&inner, CountEnvelope(Arc::clone(&mailbox_dispatches)));

    let mut scope = scope_state(&inner);
    let owned = OwnedTasks::new(Arc::clone(&inner));
    let options = <TestActor as ActorConfig>::Options::default()
        .with_max_in_flight(NonZeroUsize::new(REPLIES).unwrap());
    let (_, _, mut scheduler) = TestActor::open(&options);
    for _ in 0..REPLIES {
        let replies_polled = Arc::clone(&replies_polled);
        scheduler.__push_interleaved(
            Seal,
            async move {
                replies_polled.fetch_add(1, Ordering::SeqCst);
            }
            .into_actor(),
        );
    }
    let mut actor = TestActor;
    // Start at replies. The old path continued to the ready mailbox.
    scheduler.state().cursor = InterleavedLane::Interleaved;
    let mut task = Context::from_waker(Waker::noop());

    {
        let mut turn = std::pin::pin!(actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &inner,
            &owned,
            &mut scheduler,
            true,
            Mode::Running,
        ));
        assert!(turn.as_mut().poll(&mut task).is_pending());
    }

    let replies_polled = replies_polled.load(Ordering::SeqCst);
    assert!(0 < replies_polled && replies_polled < REPLIES);
    assert_eq!(mailbox_dispatches.load(Ordering::SeqCst), 0);
    assert!(scheduler.state().has_interleaved());
    assert_eq!(scheduler.state().cursor, InterleavedLane::ChildExit);
}

// Drain must observe lifecycle and child work before completion.
// Otherwise a ready barrier can skip accepted supervision events.
#[tokio::test]
async fn drain_priority_precedes_owned_completion() {
    let (inner, mut inbox) = test_actor_inner(1);
    let control = &inner.control;
    assert_eq!(control.request(Shutdown::Drain), ShutdownStatus::Requested);

    let mut scope = scope_state(&inner);
    let child = ChildId::invalid_for_test();
    scope.children.publish(ChildExit::new(
        child,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    ));

    let owned = OwnedTasks::new(Arc::clone(&inner));
    owned.close();
    let options =
        <TestActor as ActorConfig>::Options::default().with_max_in_flight(NonZeroUsize::MIN);
    let (_, _, mut scheduler) = TestActor::open(&options);
    let mut actor = TestActor;

    assert!(matches!(
        drain_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &inner,
            &owned,
            &mut scheduler,
            false,
        )
        .await,
        DrainTurn::Scheduled(SchedulerTurn::LifecycleHint)
    ));

    let turn = drain_turn(
        &mut actor,
        &mut scope,
        &mut inbox,
        &inner,
        &owned,
        &mut scheduler,
        false,
    )
    .await;
    let DrainTurn::Scheduled(SchedulerTurn::Child(event)) = turn else {
        panic!("ready child exit must precede owned completion");
    };
    assert_eq!(event.child(), &child);

    assert!(matches!(
        drain_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &inner,
            &owned,
            &mut scheduler,
            false,
        )
        .await,
        DrainTurn::RepliesFinished
    ));
}

#[tokio::test]
async fn child_kill_commits_before_actor_work_is_dropped() {
    // Active replies and queued envelopes may both run arbitrary destructors.
    // Observing the child mode from each Drop rejects any teardown that merely
    // waits for children after clearing local work instead of cancelling first.
    let child_owner = spawn::<TestActor>(());
    let child_ref = child_owner.actor_ref();
    let child_inner = Arc::clone(&child_ref.0);

    let active_observed_kill = Arc::new(AtomicBool::new(false));
    let queued_observed_kill = Arc::new(AtomicBool::new(false));
    let (inner, mut inbox) = test_actor_inner(1);
    enqueue_test_envelope(
        &inner,
        ChildKillDropProbe {
            child: Arc::clone(&child_inner),
            observed_kill: Arc::clone(&queued_observed_kill),
        },
    );
    assert_eq!(
        inner.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    let mut scope = scope_state(&inner);
    scope.children.insert_ref(&child_ref);
    let owned = OwnedTasks::new(Arc::clone(&inner));
    let options =
        <TestActor as ActorConfig>::Options::default().with_max_in_flight(NonZeroUsize::MIN);
    let (_, _, mut scheduler) = TestActor::open(&options);
    owned.spawn(ChildKillDropProbe {
        child: child_inner,
        observed_kill: Arc::clone(&active_observed_kill),
    });

    assert_eq!(
        kill_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await,
        ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated)
    );
    assert!(active_observed_kill.load(Ordering::SeqCst));
    assert!(queued_observed_kill.load(Ordering::SeqCst));
    drop(child_owner);
}

#[tokio::test]
async fn stop_discard_observes_kill_from_each_envelope_drop() {
    // The first destructor upgrades Stop to Kill. Leaving the second envelope
    // queued proves teardown observes the transition between individual Drops,
    // before hard teardown takes ownership of the remaining work.
    let (inner, mut inbox) = test_actor_inner(2);
    let second_dropped = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::RequestKill(Arc::downgrade(&inner)),
    );
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::MarkDropped(Arc::clone(&second_dropped)),
    );
    assert_eq!(
        inner.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    assert_eq!(
        close_and_discard(&mut inbox, &inner.control, Mode::Stopping).await,
        DiscardOutcome::ModeChanged
    );
    assert_eq!(inner.control.mode(), Mode::Killing);
    assert!(!second_dropped.load(Ordering::SeqCst));

    assert_eq!(
        close_and_discard(&mut inbox, &inner.control, Mode::Killing).await,
        DiscardOutcome::Complete
    );
    assert!(second_dropped.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_discard_yields_after_its_fixed_drop_budget() {
    // One manual poll must stop before the seventeenth Drop, and the next must
    // complete it. This locks the exact cooperative boundary while rejecting
    // both an unbounded drain and an implementation that yields too frequently.
    let (inner, mut inbox) = test_actor_inner(TEARDOWN_DROP_BUDGET + 1);
    for _ in 0..TEARDOWN_DROP_BUDGET {
        enqueue_test_envelope(&inner, TeardownEnvelope::Noop);
    }

    let last_dropped = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::MarkDropped(Arc::clone(&last_dropped)),
    );
    assert_eq!(
        inner.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    let discard = close_and_discard(&mut inbox, &inner.control, Mode::Stopping);
    tokio::pin!(discard);
    let mut task = Context::from_waker(Waker::noop());

    assert_eq!(discard.as_mut().poll(&mut task), Poll::Pending);
    assert!(!last_dropped.load(Ordering::SeqCst));
    assert_eq!(
        discard.as_mut().poll(&mut task),
        Poll::Ready(DiscardOutcome::Complete)
    );
    assert!(last_dropped.load(Ordering::SeqCst));
}

struct CapacityPanicWake(AtomicUsize);

impl Wake for CapacityPanicWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional capacity notification panic");
    }
}

// Tokio wakes capacity waiters while closing its receiver.
// ActorInbox contains that panic before discarding accepted envelopes.
// Each envelope then receives its own containment boundary.
#[test]
fn inbox_drop_contains_each_envelope_drop() {
    let (inner, inbox) = test_actor_inner(2);
    let panic_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(&inner, TeardownEnvelope::Panic(Arc::clone(&panic_dropped)));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::TrackDrop {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&tail_dropped_while_unwinding),
        },
    );
    let wakes = Arc::new(CapacityPanicWake(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);
    let mut reserve = Box::pin(inner.sender.reserve_owned());
    assert!(matches!(reserve.as_mut().poll(&mut task), Poll::Pending));
    assert_eq!(
        inner.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    drop(inbox);
    assert!(panic_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(inner.control.mode(), Mode::Killing);
}

#[test]
fn unbounded_inbox_drop_contains_each_envelope_drop() {
    let options = <UnboundedTestActor as ActorConfig>::Options::default();
    let (inner, inbox, _scheduler) = ActorInner::<UnboundedTestActor>::open(&options);
    let panic_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(&inner, TeardownEnvelope::Panic(Arc::clone(&panic_dropped)));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::TrackDrop {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&tail_dropped_while_unwinding),
        },
    );
    assert_eq!(
        inner.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    drop(inbox);
    assert!(panic_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(inner.control.mode(), Mode::Killing);
}
