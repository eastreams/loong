use std::{
    future::Future,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use tokio::sync::mpsc;

use crate::{
    ActorScope, ChildExit, ChildId, ExitReason, ExitStatus, IntoActorFuture, Shutdown,
    SubtreeStatus,
    mailbox::{ActorInbox, ActorInner, Control, Envelope, Mode},
    owned::OwnedTasks,
    scheduler::ReplyScheduler,
    transport::MessageConfig,
};

use super::super::{ChildSet, OrdinaryLane, ScopeState, Turn, TurnCursor, actor_turn};
use super::{CountEnvelope, TestActor, enqueue_test_envelope, scope_state, test_actor_inner};

/// Owns all inputs for focused actor-turn tests.
struct ActorTurnFixture {
    actor: TestActor,
    state: ScopeState<TestActor>,
    inbox: ActorInbox<TestActor>,
    supervisor_rx: mpsc::UnboundedReceiver<ChildExit>,
    inner: Arc<ActorInner<TestActor>>,
    owned: OwnedTasks<TestActor>,
    scheduler: ReplyScheduler<TestActor>,
    cursor: TurnCursor,
}

impl ActorTurnFixture {
    fn new(mailbox_capacity: usize, max_interleaved: NonZeroUsize) -> Self {
        let (inner, inbox) = test_actor_inner(mailbox_capacity);
        let (state, supervisor_rx) = scope_state(&inner, ChildSet::default());
        let owned = OwnedTasks::new(Arc::clone(&inner));
        Self {
            actor: TestActor,
            state,
            inbox,
            supervisor_rx,
            inner,
            owned,
            scheduler: ReplyScheduler::new(max_interleaved),
            cursor: TurnCursor::default(),
        }
    }

    fn enqueue(&self, envelope: impl Envelope<TestActor> + 'static) {
        enqueue_test_envelope(&self.inner, envelope);
    }

    async fn next(&mut self, expected_mode: Mode) -> Turn {
        actor_turn(
            &mut self.actor,
            &mut self.state,
            &mut self.inbox,
            &mut self.supervisor_rx,
            &self.inner,
            &self.owned,
            &mut self.scheduler,
            true,
            expected_mode,
            &mut self.cursor,
        )
        .await
    }

    /// Polls once for parking and wake assertions.
    fn poll_once(&mut self, expected_mode: Mode, task: &mut Context<'_>) -> Poll<Turn> {
        let turn = actor_turn(
            &mut self.actor,
            &mut self.state,
            &mut self.inbox,
            &mut self.supervisor_rx,
            &self.inner,
            &self.owned,
            &mut self.scheduler,
            true,
            expected_mode,
            &mut self.cursor,
        );
        tokio::pin!(turn);
        turn.poll(task)
    }
}

enum BatchBoundary {
    Shutdown(Shutdown),
    Exclusive,
    Interleaved,
}

struct BatchBoundaryEnvelope {
    dispatched: Arc<AtomicUsize>,
    boundary: BatchBoundary,
}

impl Envelope<TestActor> for BatchBoundaryEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        scheduler: &mut ReplyScheduler<TestActor>,
        inner: &Arc<ActorInner<TestActor>>,
    ) {
        self.dispatched.fetch_add(1, Ordering::SeqCst);
        match self.boundary {
            BatchBoundary::Shutdown(shutdown) => {
                inner.control.request(shutdown);
            }
            BatchBoundary::Exclusive => {
                scheduler.push_exclusive(std::future::pending::<()>().into_actor());
            }
            BatchBoundary::Interleaved => {
                scheduler.push_interleaved(async {}.into_actor());
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

    fixture
        .state
        .supervisor_tx
        .send(ChildExit::new(
            ChildId::invalid_for_test(),
            ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
        ))
        .unwrap();

    let interleaved_completed = Arc::new(AtomicBool::new(false));
    fixture.scheduler.push_interleaved({
        let completed = Arc::clone(&interleaved_completed);
        async move {
            completed.store(true, Ordering::SeqCst);
        }
        .into_actor()
    });
    let mut mailbox_turns = 0;
    let mut reply_turns = 0;
    let mut child_turns = 0;

    for _ in 0..3 {
        match fixture.next(Mode::Running).await {
            Turn::MailboxProgress => mailbox_turns += 1,
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
    assert_eq!(
        mailbox_dispatches.load(Ordering::SeqCst),
        mailbox_dispatch_budget
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
        fixture.enqueue(BatchBoundaryEnvelope {
            dispatched: Arc::clone(&dispatched),
            boundary: BatchBoundary::Shutdown(shutdown),
        });
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
        let mut task = Context::from_waker(Waker::noop());

        assert!(matches!(
            fixture.poll_once(Mode::Running, &mut task),
            Poll::Ready(Turn::LifecycleHint)
        ));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
        assert!(!fixture.inbox.is_empty());
    }
}

// Drain inherits the running cursor and dispatch budget.
// Other lanes must win before its mailbox batch resumes.
#[tokio::test]
async fn drain_rotates_then_uses_the_configured_mailbox_budget() {
    let mailbox_dispatch_budget = <TestActor as MessageConfig>::MAILBOX_DISPATCH_BUDGET.get();
    let dispatched = Arc::new(AtomicUsize::new(0));
    let mut fixture =
        ActorTurnFixture::new(mailbox_dispatch_budget + 2, NonZeroUsize::new(2).unwrap());
    fixture.scheduler.push_interleaved(async {}.into_actor());
    fixture
        .state
        .supervisor_tx
        .send(ChildExit::new(
            ChildId::invalid_for_test(),
            ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
        ))
        .unwrap();
    fixture.enqueue(BatchBoundaryEnvelope {
        dispatched: Arc::clone(&dispatched),
        boundary: BatchBoundary::Shutdown(Shutdown::Drain),
    });
    for _ in 0..=mailbox_dispatch_budget {
        fixture.enqueue(CountEnvelope(Arc::clone(&dispatched)));
    }

    assert!(matches!(
        fixture.next(Mode::Running).await,
        Turn::LifecycleHint
    ));
    assert_eq!(fixture.inner.control.mode(), Mode::Draining);
    fixture.inner.control.actor_notified().await;
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        Turn::ReplyProgress
    ));
    assert!(matches!(fixture.next(Mode::Draining).await, Turn::Child(_)));
    assert!(matches!(
        fixture.next(Mode::Draining).await,
        Turn::MailboxProgress
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
            Poll::Ready(Turn::MailboxProgress)
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
    fixture.cursor.next_ordinary = OrdinaryLane::Interleaved;
    fixture.enqueue(BatchBoundaryEnvelope {
        dispatched: Arc::clone(&dispatched),
        boundary: BatchBoundary::Interleaved,
    });
    let mut task = Context::from_waker(Waker::noop());

    assert!(matches!(
        fixture.poll_once(Mode::Running, &mut task),
        Poll::Ready(Turn::MailboxProgress)
    ));
    assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    assert!(fixture.scheduler.has_interleaved());
    assert!(matches!(
        fixture.poll_once(Mode::Running, &mut task),
        Poll::Ready(Turn::ReplyProgress)
    ));
    assert!(!fixture.scheduler.has_interleaved());
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
