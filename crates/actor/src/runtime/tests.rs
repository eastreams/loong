use std::{
    future::Future,
    marker::PhantomPinned,
    num::NonZeroUsize,
    panic,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

use tokio::sync::{mpsc, oneshot};

use crate::{
    Actor, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, ExitStatus, IntoActorFuture,
    Message, Shutdown, ShutdownStatus, SubtreeStatus, SyncHandler,
    mailbox::{ActorMailbox, Control, DynEnvelope, Envelope, Mode},
    owned::OwnedTasks,
    scheduler::ReplyScheduler,
    spawn,
};

use super::{
    ActorOwner, ActorTask, ActorWorkGuard, ActorWorkState, ChildSet, DiscardOutcome, ExitGuard,
    OrdinaryLane, OwnedActor, ScopeState, TEARDOWN_DROP_BUDGET, Turn, TurnCursor, Work, actor_turn,
    await_actor_work, close_and_discard, graceful_finish, handle_child_exit, kill_actor, run_actor,
};

struct TestActor;

impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
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
    status: Option<ExitStatus>,
    dropped: Arc<AtomicBool>,
}

impl Future for ActorFrameDropProbe {
    type Output = ExitStatus;

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        match self.status {
            Some(status) => Poll::Ready(status),
            None => Poll::Pending,
        }
    }
}

impl Drop for ActorFrameDropProbe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        panic!("intentional actor frame drop panic");
    }
}

struct ActorWorkDropProbe {
    kill: Option<Arc<Control>>,
    panic_on_poll: bool,
    polled: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    _pin: PhantomPinned,
}

impl Future for ActorWorkDropProbe {
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

impl Drop for ActorWorkDropProbe {
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

impl Actor for AbortChildParent {
    type SpawnArgs = oneshot::Sender<ExitStatus>;

    async fn init(
        child_exit: oneshot::Sender<ExitStatus>,
        scope: &mut ActorScope<'_, Self>,
    ) -> Self {
        let child = scope.spawn_child::<TestActor>(());
        scope
            .state
            .children
            .actors
            .get(child.id().key())
            .expect("the parent state owns its returned child")
            .join
            .as_ref()
            .expect("a spawned child owns its task")
            .abort();

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

struct Ping;

impl Message for Ping {
    type Reply = ();
}

impl SyncHandler<Ping> for AbortChildParent {
    fn handle(&mut self, _message: Ping, _scope: &mut ActorScope<Self>) {}
}

struct ControlledChildExit {
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    completed: Option<oneshot::Sender<()>>,
}

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

impl Envelope<TestActor> for CountEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PanicEnvelope;

impl Envelope<TestActor> for PanicEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        panic!("intentional dispatch panic");
    }
}

struct ChildKillDropProbe {
    child: Arc<Control>,
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
        let mode = self.child.mode();
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
        _owned: &OwnedTasks,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        unreachable!("Kill discards queued envelopes")
    }
}

enum TeardownEnvelope {
    Noop,
    Panic(Arc<AtomicBool>),
    RequestKill(Arc<Control>),
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
                control.request(Shutdown::Kill);
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

impl Envelope<TestActor> for TeardownEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        unreachable!("teardown discards queued envelopes")
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
                kill: kill_on_poll.then(|| Arc::clone(&control)),
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

// Tokio retains an escaped task payload inside JoinError.
// Waiting must consume that payload without another unwind.
#[tokio::test]
async fn owned_actor_wait_contains_join_panic_payload_destruction() {
    let control = Control::new();
    let aborted = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
    assert_eq!(control.finish(aborted), aborted);
    let payload_dropped = Arc::new(AtomicBool::new(false));
    let task_payload_dropped = Arc::clone(&payload_dropped);
    let mut actor = OwnedActor {
        control,
        join: Some(tokio::spawn(async move {
            panic::panic_any(CascadingPanicPayload(task_payload_dropped))
        })),
    };

    assert_eq!(actor.wait().await, aborted);
    assert!(payload_dropped.load(Ordering::SeqCst));
}

// Final frame Drop occurs before terminal publication.
// Its panic loses only when a committed Kill already won.
#[test]
fn actor_task_contains_final_frame_drop_panic() {
    for (shutdown, expected_reason) in [
        (None, ExitReason::Panicked),
        (Some(Shutdown::Kill), ExitReason::Killed),
    ] {
        let control = Control::new();
        if let Some(shutdown) = shutdown {
            assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut actor_task = Box::pin(ActorTask::new(
            Box::pin(ActorFrameDropProbe {
                status: Some(ExitStatus::new(
                    ExitReason::Stopped,
                    SubtreeStatus::Terminated,
                )),
                dropped: Arc::clone(&dropped),
            }),
            ExitGuard::new(Arc::clone(&control), None),
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
    let control = Control::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let actor_task = ActorTask::new(
        Box::pin(ActorFrameDropProbe {
            status: None,
            dropped: Arc::clone(&dropped),
        }),
        ExitGuard::new(Arc::clone(&control), None),
    );

    drop(actor_task);

    assert!(dropped.load(Ordering::SeqCst));
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

    let killing = Control::new();
    assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
    drop(ExitGuard::new(Arc::clone(&killing), None));
    assert_eq!(killing.exit_status(), Some(expected));

    let failing = Control::new();
    failing.begin_failure();
    drop(ExitGuard::new(Arc::clone(&failing), None));
    assert_eq!(failing.exit_status(), Some(expected));
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
        let ActorOwner {
            actor_ref: _child_ref,
            owned: child,
        } = spawn::<TestActor>(());
        child
            .join
            .as_ref()
            .expect("a spawned child owns its task")
            .abort();
        let mut children = ChildSet::default();
        children.insert(child);

        let (mailbox, inbox) = ActorMailbox::<TestActor>::channel(1);
        let control = Arc::clone(&mailbox.control);
        let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
        let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
        let scope = ScopeState {
            actor_ref,
            control: Arc::clone(&control),
            children,
            supervisor_tx,
        };

        match exit {
            ParentExit::Shutdown(shutdown) => {
                assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
            }
            ParentExit::Panic => mailbox.sender.try_send(Box::new(PanicEnvelope)).unwrap(),
        }

        let task = ActorTask::new(
            Box::pin(run_actor::<TestActor>(
                (),
                scope,
                inbox,
                supervisor_rx,
                mailbox,
                NonZeroUsize::MIN,
            )),
            ExitGuard::new(Arc::clone(&control), None),
        );
        let status = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("parent teardown must finish");
        let expected = ExitStatus::new(expected_reason, SubtreeStatus::Unconfirmed);

        assert_eq!(status, expected);
        assert_eq!(control.exit_status(), Some(expected));
    }
}

// The generation is why ChildSet uses SlotMap instead of Slab.
// A stale exit must not remove a later child reusing the same slot.
#[test]
fn stale_child_exit_cannot_remove_a_reused_slot() {
    let mut children = ChildSet::default();
    let first = children.insert(OwnedActor {
        control: Control::new(),
        join: None,
    });
    assert!(children.remove(&ChildExit::new(
        first,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    )));

    let second = children.insert(OwnedActor {
        control: Control::new(),
        join: None,
    });
    assert_ne!(first, second);
    assert!(!children.remove(&ChildExit::new(
        first,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    )));
    assert_eq!(children.len(), 1);
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
        let mut children = ChildSet::default();
        let child_id = children.insert(OwnedActor {
            control: Control::new(),
            join: None,
        });

        let (mailbox, _inbox) = ActorMailbox::<CountChildExit>::channel(1);
        let control = Arc::clone(&mailbox.control);
        let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
        let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
        let mut scope = ScopeState {
            actor_ref,
            control: Arc::clone(&control),
            children,
            supervisor_tx,
        };
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
                &control,
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
        let mut children = ChildSet::default();
        let child_id = children.insert(OwnedActor {
            control: Control::new(),
            join: None,
        });

        let (mailbox, _inbox) = ActorMailbox::<ControlledChildExit>::channel(1);
        let control = Arc::clone(&mailbox.control);
        let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
        let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
        let mut scope = ScopeState {
            actor_ref,
            control: Arc::clone(&control),
            children,
            supervisor_tx,
        };
        let mut actor = ControlledChildExit {
            entered: Some(entered_tx),
            release: Some(release_rx),
            completed: Some(completed_tx),
        };
        let controller = Arc::clone(&control);

        let (work, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                handle_child_exit(
                    &mut actor,
                    &mut scope,
                    ChildExit::new(
                        child_id,
                        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
                    ),
                    &control,
                ),
                async move {
                    entered_rx.await.unwrap();
                    assert_eq!(controller.request(shutdown), ShutdownStatus::Requested);
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
    let (mailbox, inbox) = ActorMailbox::<TestActor>::channel(1);
    let permit = mailbox
        .sender
        .clone()
        .reserve_owned()
        .await
        .expect("the test mailbox is open");
    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
    let scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        supervisor_tx,
    };
    assert_eq!(control.request(Shutdown::Drain), ShutdownStatus::Requested);

    let status = tokio::time::timeout(
        Duration::from_secs(1),
        run_actor::<TestActor>(
            (),
            scope,
            inbox,
            supervisor_rx,
            mailbox,
            NonZeroUsize::new(1).unwrap(),
        ),
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
    let child_control = Control::new();
    let mut children = ChildSet::default();
    children.insert(OwnedActor {
        control: Arc::clone(&child_control),
        join: None,
    });

    let (mailbox, _inbox) = ActorMailbox::<TestActor>::channel(1);
    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children,
        supervisor_tx,
    };
    let mut actor = TestActor;

    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);
    assert!(matches!(
        graceful_finish(
            &mut actor,
            &mut scope,
            &control,
            Shutdown::Stop,
            ExitReason::Stopped,
        )
        .await,
        Work::Killed
    ));
    assert_eq!(child_control.mode(), Mode::Running);
}

#[tokio::test]
async fn ordinary_cursor_visits_each_actor_source_before_repeating_mailbox() {
    // Each actor source must win before mailbox work repeats.
    let mailbox_dispatches = Arc::new(AtomicUsize::new(0));
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(5);
    for _ in 0..5 {
        mailbox
            .sender
            .try_send(
                Box::new(CountEnvelope(Arc::clone(&mailbox_dispatches))) as DynEnvelope<TestActor>
            )
            .unwrap();
    }

    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        supervisor_tx: supervisor_tx.clone(),
    };
    supervisor_tx
        .send(ChildExit::new(
            ChildId::invalid_for_test(),
            ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
        ))
        .unwrap();

    let interleaved_completed = Arc::new(AtomicBool::new(false));
    let owned = OwnedTasks::new(Arc::clone(&control));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(2).unwrap());
    scheduler.push_interleaved({
        let completed = Arc::clone(&interleaved_completed);
        async move {
            completed.store(true, Ordering::SeqCst);
        }
        .into_actor()
    });
    let mut actor = TestActor;
    // Start at child work to prove the cursor wraps across all sources.
    let mut cursor = TurnCursor {
        next_ordinary: OrdinaryLane::ChildExit,
    };
    let mut mailbox_turns = 0;
    let mut reply_turns = 0;
    let mut child_turns = 0;

    for _ in 0..3 {
        match actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &control,
            &owned,
            &mut scheduler,
            true,
            Mode::Running,
            &mut cursor,
        )
        .await
        {
            Turn::Message => mailbox_turns += 1,
            Turn::ReplyProgress => reply_turns += 1,
            Turn::Child(event) => {
                assert_eq!(
                    event.status(),
                    ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated)
                );
                child_turns += 1;
            }
            Turn::LifecycleHint => panic!("the lifecycle mode changed unexpectedly"),
            Turn::RepliesFinished => panic!("running work cannot finish reply scheduling"),
            Turn::InboxClosed => panic!("the mailbox closed unexpectedly"),
        }
    }

    assert_eq!(mailbox_turns, 1);
    assert_eq!(reply_turns, 1);
    assert_eq!(child_turns, 1);
    assert!(interleaved_completed.load(Ordering::SeqCst));
    assert_eq!(mailbox_dispatches.load(Ordering::SeqCst), 1);
    assert_eq!(inbox.len(), 4);
}

// A truncated reply sweep must yield before polling another ready lane.
// Otherwise ready mailbox traffic can erase the cooperative boundary.
#[test]
fn truncated_reply_sweep_yields_before_ready_mailbox() {
    // Seventeen ready replies equal the poll budget plus one.
    const REPLIES: usize = 17;

    let mailbox_dispatches = Arc::new(AtomicUsize::new(0));
    let replies_polled = Arc::new(AtomicUsize::new(0));
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    mailbox
        .sender
        .try_send(
            Box::new(CountEnvelope(Arc::clone(&mailbox_dispatches))) as DynEnvelope<TestActor>
        )
        .unwrap();

    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        supervisor_tx,
    };
    let owned = OwnedTasks::new(Arc::clone(&control));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(REPLIES).unwrap());
    for _ in 0..REPLIES {
        let replies_polled = Arc::clone(&replies_polled);
        scheduler.push_interleaved(
            async move {
                replies_polled.fetch_add(1, Ordering::SeqCst);
            }
            .into_actor(),
        );
    }
    let mut actor = TestActor;
    // Start at replies. The old path continued to the ready mailbox.
    let mut cursor = TurnCursor {
        next_ordinary: OrdinaryLane::Interleaved,
    };
    let mut task = Context::from_waker(Waker::noop());

    {
        let mut turn = std::pin::pin!(actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &control,
            &owned,
            &mut scheduler,
            true,
            Mode::Running,
            &mut cursor,
        ));
        assert!(turn.as_mut().poll(&mut task).is_pending());
    }

    let replies_polled = replies_polled.load(Ordering::SeqCst);
    assert!(0 < replies_polled && replies_polled < REPLIES);
    assert_eq!(mailbox_dispatches.load(Ordering::SeqCst), 0);
    assert_eq!(inbox.len(), 1);
    assert!(scheduler.has_interleaved());
    assert_eq!(cursor.next_ordinary, OrdinaryLane::ChildExit);
}

// Parent runtime state keeps this receiver open.
// Treat early closure as corruption; a hint would spin forever.
#[tokio::test]
#[should_panic(expected = "child-exit receiver closed while parent runtime was alive")]
async fn closed_child_exit_channel_is_an_invariant_failure() {
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        supervisor_tx,
    };
    supervisor_rx.close();

    let owned = OwnedTasks::new(Arc::clone(&control));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::MIN);
    let mut actor = TestActor;
    let mut cursor = TurnCursor {
        next_ordinary: OrdinaryLane::ChildExit,
    };

    actor_turn(
        &mut actor,
        &mut scope,
        &mut inbox,
        &mut supervisor_rx,
        &control,
        &owned,
        &mut scheduler,
        true,
        Mode::Running,
        &mut cursor,
    )
    .await;
}

// Drain must absorb queued child exits before its completion barrier.
// Otherwise cleanup skips events already accepted by supervision.
#[tokio::test]
async fn drain_absorbs_ready_child_exit_before_owned_completion() {
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    let control = Arc::clone(&mailbox.control);
    assert_eq!(control.request(Shutdown::Drain), ShutdownStatus::Requested);
    control.actor_notified().await;

    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        supervisor_tx: supervisor_tx.clone(),
    };
    let child = ChildId::invalid_for_test();
    supervisor_tx
        .send(ChildExit::new(
            child,
            ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
        ))
        .unwrap();

    let owned = OwnedTasks::new(Arc::clone(&control));
    owned.close();
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::MIN);
    let mut actor = TestActor;
    let mut cursor = TurnCursor::default();

    let turn = actor_turn(
        &mut actor,
        &mut scope,
        &mut inbox,
        &mut supervisor_rx,
        &control,
        &owned,
        &mut scheduler,
        false,
        Mode::Draining,
        &mut cursor,
    )
    .await;
    let Turn::Child(event) = turn else {
        panic!("ready child exit must precede owned completion");
    };
    assert_eq!(event.child(), &child);

    assert!(matches!(
        actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &control,
            &owned,
            &mut scheduler,
            false,
            Mode::Draining,
            &mut cursor,
        )
        .await,
        Turn::RepliesFinished
    ));
}

// Roots and children share OwnedActor.
// Cancelling one wait must retain their private completion barrier.
// The second wait must reuse the same JoinHandle.
#[tokio::test]
async fn cancelled_owned_actor_wait_retains_its_join_handle() {
    let ActorOwner {
        actor_ref: _actor_ref,
        mut owned,
    } = spawn::<TestActor>(());
    {
        let mut wait = Box::pin(owned.wait());
        let mut task = Context::from_waker(Waker::noop());
        assert!(wait.as_mut().poll(&mut task).is_pending());
    }

    assert!(owned.join.is_some());
    assert_eq!(
        owned.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    assert_eq!(
        owned.wait().await,
        ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated)
    );
}

#[tokio::test]
async fn child_kill_commits_before_actor_work_is_dropped() {
    // Active replies and queued envelopes may both run arbitrary destructors.
    // Observing the child mode from each Drop rejects any teardown that merely
    // waits for children after clearing local work instead of cancelling first.
    let ActorOwner {
        actor_ref: _child_ref,
        owned: child,
    } = spawn::<TestActor>(());
    let child_control = Arc::clone(&child.control);
    let mut children = ChildSet::default();
    children.insert(child);

    let active_observed_kill = Arc::new(AtomicBool::new(false));
    let queued_observed_kill = Arc::new(AtomicBool::new(false));
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    mailbox
        .sender
        .try_send(Box::new(ChildKillDropProbe {
            child: Arc::clone(&child_control),
            observed_kill: Arc::clone(&queued_observed_kill),
        }))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), mailbox.control.subscribe_mode());
    let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ScopeState {
        actor_ref,
        control: Arc::clone(&mailbox.control),
        children,
        supervisor_tx,
    };
    let owned = OwnedTasks::new(Arc::clone(&scope.control));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(1).unwrap());
    owned.spawn(ChildKillDropProbe {
        child: child_control,
        observed_kill: Arc::clone(&active_observed_kill),
    });

    assert_eq!(
        kill_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await,
        ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated)
    );
    assert!(active_observed_kill.load(Ordering::SeqCst));
    assert!(queued_observed_kill.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stop_discard_observes_kill_from_each_envelope_drop() {
    // The first destructor upgrades Stop to Kill. Leaving the second envelope
    // queued proves teardown observes the transition between individual Drops,
    // before hard teardown takes ownership of the remaining work.
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(2);
    let second_dropped = Arc::new(AtomicBool::new(false));
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::RequestKill(Arc::clone(
            &mailbox.control,
        ))))
        .unwrap();
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::MarkDropped(Arc::clone(
            &second_dropped,
        ))))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    assert_eq!(
        close_and_discard(&mut inbox, &mailbox.control, Mode::Stopping).await,
        DiscardOutcome::ModeChanged
    );
    assert_eq!(mailbox.control.mode(), Mode::Killing);
    assert!(!second_dropped.load(Ordering::SeqCst));
    assert_eq!(inbox.len(), 1);

    assert_eq!(
        close_and_discard(&mut inbox, &mailbox.control, Mode::Killing).await,
        DiscardOutcome::Complete
    );
    assert!(second_dropped.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_discard_yields_after_its_fixed_drop_budget() {
    // One manual poll must stop before the seventeenth Drop, and the next must
    // complete it. This locks the exact cooperative boundary while rejecting
    // both an unbounded drain and an implementation that yields too frequently.
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(TEARDOWN_DROP_BUDGET + 1);
    for _ in 0..TEARDOWN_DROP_BUDGET {
        mailbox
            .sender
            .try_send(Box::new(TeardownEnvelope::Noop))
            .unwrap();
    }

    let last_dropped = Arc::new(AtomicBool::new(false));
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::MarkDropped(Arc::clone(
            &last_dropped,
        ))))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    let discard = close_and_discard(&mut inbox, &mailbox.control, Mode::Stopping);
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

// A queued destructor panic must not unwind from hard teardown.
// The next accepted envelope must still be destroyed.
// Its unwind state rejects one aggregate panic boundary.
#[tokio::test]
async fn queued_discard_contains_each_envelope_drop() {
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(2);
    let panic_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::Panic(Arc::clone(
            &panic_dropped,
        ))))
        .unwrap();
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::TrackDrop {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&tail_dropped_while_unwinding),
        }))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    assert_eq!(
        close_and_discard(&mut inbox, &mailbox.control, Mode::Killing).await,
        DiscardOutcome::Complete
    );
    assert!(panic_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(mailbox.control.mode(), Mode::Killing);
}
