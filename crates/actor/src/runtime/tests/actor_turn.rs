use std::{
    future::Future,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use crate::{
    Actor, ActorConfig, ActorScope, ChildExit, ChildId, ExitReason, ExitStatus, HasMailbox,
    IntoActorFuture, ReplySchedulingConfig, Shutdown, SubtreeStatus,
    mailbox::{ActorInbox, ActorInner, Control, Envelope, Mode},
    owned::OwnedTasks,
    scheduling::{
        ActorScheduler, InterleavedLane, InterleavedProfile, RuntimeInterleavedScheduler,
        RuntimeScheduler, SchedulerTurn,
    },
    supervision::runtime::tests::ChildrenFixture,
    transport::MessageConfig,
};

use super::super::{ScopeState, actor_turn};
use super::{CountEnvelope, TestActor, enqueue_test_envelope, scope_state, test_actor_inner};

/// Owns all inputs for focused actor-turn tests.
struct ActorTurnFixture<A: Actor> {
    actor: A,
    state: ScopeState<A>,
    inbox: ActorInbox<A>,
    inner: Arc<ActorInner<A>>,
    owned: OwnedTasks<A>,
    scheduler: ActorScheduler<A>,
}

impl<A: Actor> ActorTurnFixture<A> {
    fn from_parts(
        actor: A,
        inner: Arc<ActorInner<A>>,
        inbox: ActorInbox<A>,
        scheduler: ActorScheduler<A>,
    ) -> Self {
        let state = scope_state(&inner);
        let owned = OwnedTasks::new(Arc::clone(&inner));
        Self {
            actor,
            state,
            inbox,
            inner,
            owned,
            scheduler,
        }
    }

    async fn next(&mut self, expected_mode: Mode) -> SchedulerTurn {
        actor_turn(
            &mut self.actor,
            &mut self.state,
            &mut self.inbox,
            &self.inner,
            &self.owned,
            &mut self.scheduler,
            true,
            expected_mode,
        )
        .await
    }

    /// Polls once for parking and wake assertions.
    fn poll_once(&mut self, expected_mode: Mode, task: &mut Context<'_>) -> Poll<SchedulerTurn> {
        let turn = actor_turn(
            &mut self.actor,
            &mut self.state,
            &mut self.inbox,
            &self.inner,
            &self.owned,
            &mut self.scheduler,
            true,
            expected_mode,
        );
        tokio::pin!(turn);
        turn.poll(task)
    }
}

impl<A: HasMailbox> ActorTurnFixture<A> {
    fn enqueue(&self, envelope: impl Envelope<A> + 'static) {
        enqueue_test_envelope(&self.inner, envelope);
    }
}

impl ActorTurnFixture<TestActor> {
    fn new(mailbox_capacity: usize, max_interleaved: NonZeroUsize) -> Self {
        let (inner, inbox) = test_actor_inner(mailbox_capacity);
        let options =
            <TestActor as ActorConfig>::Options::default().with_max_in_flight(max_interleaved);
        Self::from_parts(TestActor, inner, inbox, TestActor::open_scheduler(&options))
    }
}

struct SerialActor;

#[crate::actor(mailbox = dynamic, mailbox_budget = 3, children = 1)]
impl Actor for SerialActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl ActorTurnFixture<SerialActor> {
    fn new_serial(mailbox_capacity: usize) -> Self {
        let capacity = NonZeroUsize::new(mailbox_capacity).expect("test capacity is nonzero");
        let options =
            <SerialActor as ActorConfig>::Options::default().with_mailbox_capacity(capacity);
        let (inner, inbox) = ActorInner::open(&options);
        Self::from_parts(
            SerialActor,
            inner,
            inbox,
            SerialActor::open_scheduler(&options),
        )
    }
}

enum BatchBoundary {
    Exclusive,
    Interleaved,
}

struct BatchBoundaryEnvelope {
    dispatched: Arc<AtomicUsize>,
    boundary: BatchBoundary,
}

struct ShutdownEnvelope {
    dispatched: Arc<AtomicUsize>,
    shutdown: Shutdown,
}

impl<A: Actor> Envelope<A> for ShutdownEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut A,
        _scope: &mut ActorScope<A>,
        _owned: &OwnedTasks<A>,
        _scheduler: &mut ActorScheduler<A>,
        inner: &Arc<ActorInner<A>>,
    ) {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        inner.control.request(self.shutdown);
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

struct ReadyExclusiveDrop {
    drops: Arc<AtomicUsize>,
    dropped_while_unwinding: Arc<AtomicBool>,
}

impl crate::ActorFuture<TestActor> for ReadyExclusiveDrop {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<'_, TestActor>,
        _task: &mut Context<'_>,
    ) -> Poll<()> {
        Poll::Ready(())
    }
}

impl Drop for ReadyExclusiveDrop {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
        panic!("intentional exclusive future drop panic");
    }
}

impl Envelope<TestActor> for BatchBoundaryEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        scheduler: &mut ActorScheduler<TestActor>,
        _inner: &Arc<ActorInner<TestActor>>,
    ) {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        match self.boundary {
            BatchBoundary::Exclusive => {
                RuntimeScheduler::push_exclusive(
                    scheduler,
                    std::future::pending::<()>().into_actor(),
                );
            }
            BatchBoundary::Interleaved => {
                RuntimeInterleavedScheduler::push_interleaved(scheduler, async {}.into_actor());
            }
        }
    }

    fn discard(self: Box<Self>, control: &Control) {
        control.drop_user_value(self);
    }
}

#[tokio::test]
async fn ordinary_cursor_visits_each_actor_source_between_mailbox_batches() {
    // Each source must win before another mailbox batch.
    let mailbox_dispatch_budget = <TestActor as MessageConfig>::MAILBOX_DISPATCH_BUDGET.get();
    let mailbox_dispatches = Arc::new(AtomicUsize::new(0));
    let mut fixture =
        ActorTurnFixture::new(mailbox_dispatch_budget + 1, NonZeroUsize::new(2).unwrap());
    for _ in 0..=mailbox_dispatch_budget {
        fixture.enqueue(CountEnvelope(Arc::clone(&mailbox_dispatches)));
    }

    fixture.state.children.publish(ChildExit::new(
        ChildId::invalid_for_test(),
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    ));

    let interleaved_completed = Arc::new(AtomicBool::new(false));
    RuntimeInterleavedScheduler::push_interleaved(&mut fixture.scheduler, {
        let completed = Arc::clone(&interleaved_completed);
        async move {
            completed.store(true, Ordering::SeqCst);
        }
        .into_actor()
    });
    assert!(matches!(
        fixture.next(Mode::Running).await,
        SchedulerTurn::Progress
    ));
    assert_eq!(
        mailbox_dispatches.load(Ordering::SeqCst),
        mailbox_dispatch_budget
    );
    assert!(!interleaved_completed.load(Ordering::SeqCst));

    assert!(matches!(
        fixture.next(Mode::Running).await,
        SchedulerTurn::Progress
    ));
    assert!(interleaved_completed.load(Ordering::SeqCst));

    let SchedulerTurn::Child(event) = fixture.next(Mode::Running).await else {
        panic!("child exits must follow the first mailbox and reply turns");
    };
    assert_eq!(
        event.status(),
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated)
    );
    assert!(!fixture.inbox.is_empty());
}

// Dispatch may synchronously commit lifecycle shutdown.
// The batch must stop before the next envelope.
#[test]
fn mailbox_batch_stops_after_dispatch_changes_lifecycle() {
    for shutdown in [Shutdown::Stop, Shutdown::Drain, Shutdown::Kill] {
        let dispatched = Arc::new(AtomicUsize::new(0));
        let mut fixture = ActorTurnFixture::new(2, NonZeroUsize::MIN);
        fixture.enqueue(ShutdownEnvelope {
            dispatched: Arc::clone(&dispatched),
            shutdown,
        });
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
        let mut task = Context::from_waker(Waker::noop());

        assert!(matches!(
            fixture.poll_once(Mode::Running, &mut task),
            Poll::Ready(SchedulerTurn::LifecycleHint)
        ));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
        assert!(!fixture.inbox.is_empty());
    }
}

#[tokio::test]
async fn completed_exclusive_drop_panic_is_contained_after_removal() {
    let mut fixture = ActorTurnFixture::new(1, NonZeroUsize::MIN);
    let drops = Arc::new(AtomicUsize::new(0));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    RuntimeScheduler::push_exclusive(
        &mut fixture.scheduler,
        ReadyExclusiveDrop {
            drops: Arc::clone(&drops),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
        },
    );

    assert!(matches!(
        fixture.next(Mode::Running).await,
        SchedulerTurn::LifecycleHint
    ));
    assert!(RuntimeScheduler::is_idle(&mut fixture.scheduler));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(fixture.inner.control.mode(), Mode::Failing);
}

// Drain inherits the running cursor and dispatch budget.
// Other lanes must win before its mailbox batch resumes.
#[tokio::test]
async fn drain_rotates_then_uses_the_configured_mailbox_budget() {
    let mailbox_dispatch_budget = <TestActor as MessageConfig>::MAILBOX_DISPATCH_BUDGET.get();
    let dispatched = Arc::new(AtomicUsize::new(0));
    let mut fixture =
        ActorTurnFixture::new(mailbox_dispatch_budget + 2, NonZeroUsize::new(2).unwrap());
    RuntimeInterleavedScheduler::push_interleaved(&mut fixture.scheduler, async {}.into_actor());
    fixture.state.children.publish(ChildExit::new(
        ChildId::invalid_for_test(),
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    ));
    fixture.enqueue(ShutdownEnvelope {
        dispatched: Arc::clone(&dispatched),
        shutdown: Shutdown::Drain,
    });
    for _ in 0..=mailbox_dispatch_budget {
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
    }

    assert!(matches!(
        fixture.next(Mode::Running).await,
        SchedulerTurn::LifecycleHint
    ));
    assert_eq!(fixture.inner.control.mode(), Mode::Draining);
    fixture.inner.control.actor_notified().await;
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        SchedulerTurn::Progress
    ));
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert!(!fixture.scheduler.state().has_interleaved());
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        SchedulerTurn::Child(_)
    ));
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        SchedulerTurn::Progress
    ));
    assert_eq!(
        dispatched.load(Ordering::SeqCst),
        mailbox_dispatch_budget + 1
    );
    assert!(!fixture.inbox.is_empty());
}

// Serial Drain inherits the running mailbox-child cursor.
// The queued child must win before mailbox dispatch resumes.
#[tokio::test]
async fn serial_drain_inherits_cursor_before_resuming_mailbox() {
    let mailbox_dispatch_budget = <SerialActor as MessageConfig>::MAILBOX_DISPATCH_BUDGET.get();
    let dispatched = Arc::new(AtomicUsize::new(0));
    let mut fixture = ActorTurnFixture::<SerialActor>::new_serial(mailbox_dispatch_budget + 2);
    fixture.state.children.publish(ChildExit::new(
        ChildId::invalid_for_test(),
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
    ));
    fixture.enqueue(ShutdownEnvelope {
        dispatched: Arc::clone(&dispatched),
        shutdown: Shutdown::Drain,
    });
    for _ in 0..=mailbox_dispatch_budget {
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
    }

    assert!(matches!(
        fixture.next(Mode::Running).await,
        SchedulerTurn::LifecycleHint
    ));
    fixture.inner.control.actor_notified().await;
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        SchedulerTurn::Child(_)
    ));
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        SchedulerTurn::Progress
    ));
    assert_eq!(
        dispatched.load(Ordering::SeqCst),
        mailbox_dispatch_budget + 1
    );
    assert!(!fixture.inbox.is_empty());
}

// Reply scheduling can block later mailbox dispatch.
// Both scheduler modes must leave later entries queued.
#[test]
fn mailbox_batch_stops_when_reply_blocks_dispatch() {
    for boundary in [BatchBoundary::Exclusive, BatchBoundary::Interleaved] {
        let dispatched = Arc::new(AtomicUsize::new(0));
        let mut fixture = ActorTurnFixture::new(2, NonZeroUsize::MIN);
        fixture.enqueue(BatchBoundaryEnvelope {
            dispatched: Arc::clone(&dispatched),
            boundary,
        });
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
        let mut task = Context::from_waker(Waker::noop());

        assert!(matches!(
            fixture.poll_once(Mode::Running, &mut task),
            Poll::Ready(SchedulerTurn::Progress)
        ));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
        assert!(!fixture.inbox.is_empty());
    }
}

// Dispatch may activate an already-visited lane.
// Reporting progress guarantees its first poll.
#[test]
fn mailbox_batch_repolls_a_new_interleaved_reply() {
    let dispatched = Arc::new(AtomicUsize::new(0));
    let mut fixture = ActorTurnFixture::new(1, NonZeroUsize::new(2).unwrap());
    fixture.scheduler.state().cursor = InterleavedLane::Interleaved;
    fixture.enqueue(BatchBoundaryEnvelope {
        dispatched: Arc::clone(&dispatched),
        boundary: BatchBoundary::Interleaved,
    });
    let mut task = Context::from_waker(Waker::noop());

    assert!(matches!(
        fixture.poll_once(Mode::Running, &mut task),
        Poll::Ready(SchedulerTurn::Progress)
    ));
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert!(fixture.scheduler.state().has_interleaved());
    assert!(matches!(
        fixture.poll_once(Mode::Running, &mut task),
        Poll::Ready(SchedulerTurn::Progress)
    ));
    assert!(!fixture.scheduler.state().has_interleaved());
}

struct ActorTurnWakeCounter(AtomicUsize);

impl Wake for ActorTurnWakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// A partial ready batch parks in the same outer turn.
// The final receive poll must re-arm mailbox wakes.
#[test]
fn partial_mailbox_batch_parks_and_registers_its_waker() {
    let dispatched = Arc::new(AtomicUsize::new(0));
    let mut fixture = ActorTurnFixture::new(2, NonZeroUsize::MIN);
    fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
    let wakes = Arc::new(ActorTurnWakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);

    assert!(fixture.poll_once(Mode::Running, &mut task).is_pending());
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);

    fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
    assert!(wakes.0.load(Ordering::SeqCst) > 0);
}
