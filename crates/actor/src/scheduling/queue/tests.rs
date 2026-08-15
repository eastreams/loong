use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use super::*;
use crate::{Actor, ActorConfig, ActorScope, IntoActorFuture, Shutdown, mailbox::ActorInner};

struct TestActor;

#[crate::actor(mailbox = 1)]
impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn test_actor_inner() -> Arc<ActorInner<TestActor>> {
    let options = <TestActor as ActorConfig>::Options::default();
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

struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct CaptureWaker {
    polls: Arc<AtomicUsize>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl Future for CaptureWaker {
    type Output = ();

    fn poll(self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        *self.waker.lock().unwrap() = Some(task.waker().clone());
        Poll::Pending
    }
}

struct NotifyFlag(AtomicBool);

struct PanicWake;

impl Wake for NotifyFlag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Wake for PanicWake {
    fn wake(self: Arc<Self>) {
        panic!("intentional actor task wake panic");
    }

    fn wake_by_ref(self: &Arc<Self>) {
        panic!("intentional actor task wake panic");
    }
}

struct IndexedPoll {
    index: usize,
    polls: Arc<Vec<AtomicUsize>>,
    completes: bool,
}

impl Future for IndexedPoll {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls[self.index].fetch_add(1, Ordering::SeqCst);
        if self.completes {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

struct PanicOnPoll {
    dropped: Arc<AtomicBool>,
    dropped_while_unwinding: Arc<AtomicBool>,
}

struct ReadyWithPanickingDrop {
    drops: Arc<AtomicUsize>,
    dropped_while_unwinding: Arc<AtomicBool>,
}

impl Future for ReadyWithPanickingDrop {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(())
    }
}

impl Drop for ReadyWithPanickingDrop {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
        panic!("intentional future drop panic");
    }
}

impl Future for PanicOnPoll {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        panic!("intentional future poll panic")
    }
}

impl Drop for PanicOnPoll {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
    }
}

struct KillOnPoll {
    actor: Arc<ActorInner<TestActor>>,
    polls: Arc<AtomicUsize>,
}

impl Future for KillOnPoll {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        self.actor.control.request(Shutdown::Kill);
        Poll::Pending
    }
}

type TestFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

fn poll_futures(
    items: &mut VecDeque<TestFuture>,
    sweep: &mut SweepState,
    control: &Control,
    task: &mut Context<'_>,
) -> InterleavedPoll {
    poll_round_robin(
        items,
        sweep,
        control,
        Mode::Running,
        task,
        |future, task| future.as_mut().poll(task),
    )
}

#[test]
fn empty_poll_keeps_the_sweep_unallocated() {
    let actor = test_actor_inner();
    let mut items = VecDeque::new();
    let mut sweep = SweepState::default();
    let mut task = Context::from_waker(Waker::noop());

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    assert!(sweep.wake.is_none());
}

#[test]
fn polling_allocates_the_sweep_until_the_queue_empties() {
    let actor = test_actor_inner();
    let polls = Arc::new(AtomicUsize::new(0));
    let mut items = VecDeque::from([Box::pin(PollCounter(Arc::clone(&polls))) as TestFuture]);
    let mut sweep = SweepState::default();
    let mut task = Context::from_waker(Waker::noop());

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    let wake = Arc::downgrade(
        sweep
            .wake
            .as_ref()
            .expect("pending work needs a sweep waker"),
    );

    items.clear();
    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    assert!(sweep.wake.is_none());
    assert!(wake.upgrade().is_none());
}

#[test]
fn proxy_contains_actor_task_wake_panic() {
    let actor = test_actor_inner();
    let polls = Arc::new(AtomicUsize::new(0));
    let proxy = Arc::new(Mutex::new(None));
    let mut items = VecDeque::from([Box::pin(CaptureWaker {
        polls,
        waker: Arc::clone(&proxy),
    }) as TestFuture]);
    let mut sweep = SweepState::default();
    let waker = Waker::from(Arc::new(PanicWake));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    proxy
        .lock()
        .unwrap()
        .as_ref()
        .expect("the future captured its proxy waker")
        .wake_by_ref();
}

#[test]
fn budget_continuation_contains_actor_task_wake_panic() {
    let actor = test_actor_inner();
    let mut items = (0..=ACTIVE_POLL_BUDGET)
        .map(|_| Box::pin(std::future::pending()) as TestFuture)
        .collect();
    let mut sweep = SweepState::default();
    let waker = Waker::from(Arc::new(PanicWake));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::BudgetExhausted
    );
}

#[test]
fn confirmation_wake_contains_actor_task_wake_panic() {
    let actor = test_actor_inner();
    let retained = Arc::new(Mutex::new(None));
    let mut items = VecDeque::from([Box::pin(CaptureWaker {
        polls: Arc::new(AtomicUsize::new(0)),
        waker: Arc::clone(&retained),
    }) as TestFuture]);
    for _ in 1..20 {
        items.push_back(Box::pin(std::future::pending()));
    }
    let mut sweep = SweepState::default();
    let mut first_task = Context::from_waker(Waker::noop());

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut first_task),
        InterleavedPoll::BudgetExhausted
    );
    retained
        .lock()
        .unwrap()
        .as_ref()
        .expect("the first future retained its proxy")
        .wake_by_ref();

    let waker = Waker::from(Arc::new(PanicWake));
    let mut second_task = Context::from_waker(&waker);
    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut second_task),
        InterleavedPoll::Pending
    );
}

#[test]
fn clearing_detaches_a_retained_proxy_waker() {
    let actor = test_actor_inner();
    let polls = Arc::new(AtomicUsize::new(0));
    let retained = Arc::new(Mutex::new(None));
    let mut items = VecDeque::from([Box::pin(CaptureWaker {
        polls,
        waker: Arc::clone(&retained),
    }) as TestFuture]);
    let mut sweep = SweepState::default();
    let notified = Arc::new(NotifyFlag(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&notified));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    items.clear();
    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );

    retained
        .lock()
        .unwrap()
        .as_ref()
        .expect("the external source retains its proxy")
        .wake_by_ref();
    assert!(!notified.0.load(Ordering::SeqCst));
}

#[test]
fn clearing_a_sweep_resets_all_state() {
    let mut sweep = SweepState {
        remaining: 3,
        generation: 7,
        wake: Some(Arc::new(SweepWaker::default())),
    };

    sweep.clear();

    assert_eq!(sweep.remaining, 0);
    assert_eq!(sweep.generation, 0);
    assert!(sweep.wake.is_none());
}

// Automatic scheduler teardown has no lifecycle handle.
// Each future still needs an independent unwind boundary.
#[test]
fn queue_drop_contains_each_future_panic() {
    let drops = Arc::new(AtomicUsize::new(0));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut queue = Queue::<TestActor>::new();
    for _ in 0..2 {
        queue.push(Box::pin(
            ReadyWithPanickingDrop {
                drops: Arc::clone(&drops),
                dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            }
            .into_actor(),
        ));
    }

    drop(queue);

    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
}

#[test]
fn budgeted_scan_resumes_at_the_unpolled_tail() {
    let actor = test_actor_inner();
    let polls: Vec<_> = (0..20).map(|_| Arc::new(AtomicUsize::new(0))).collect();
    let mut items: VecDeque<TestFuture> = polls
        .iter()
        .map(|count| Box::pin(PollCounter(Arc::clone(count))) as TestFuture)
        .collect();
    let mut sweep = SweepState::default();
    let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::BudgetExhausted
    );
    assert!(
        polls[..ACTIVE_POLL_BUDGET]
            .iter()
            .all(|count| count.load(Ordering::SeqCst) == 1)
    );
    assert!(
        polls[ACTIVE_POLL_BUDGET..]
            .iter()
            .all(|count| count.load(Ordering::SeqCst) == 0)
    );
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) == 1));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
}

#[test]
fn future_wake_coalesced_with_continuation_starts_another_sweep() {
    let actor = test_actor_inner();
    let first_polls = Arc::new(AtomicUsize::new(0));
    let first_waker = Arc::new(Mutex::new(None));
    let mut items = VecDeque::from([Box::pin(CaptureWaker {
        polls: Arc::clone(&first_polls),
        waker: Arc::clone(&first_waker),
    }) as TestFuture]);
    for _ in 1..20 {
        items.push_back(Box::pin(std::future::pending()));
    }
    let mut sweep = SweepState::default();
    let notified = Arc::new(NotifyFlag(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&notified));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::BudgetExhausted
    );
    first_waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
    assert!(notified.0.swap(false, Ordering::SeqCst));

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    assert!(notified.0.swap(false, Ordering::SeqCst));

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::BudgetExhausted
    );
    assert_eq!(first_polls.load(Ordering::SeqCst), 2);
}

// Completion at the budget edge must still yield.
#[test]
fn completion_at_budget_cut_yields_before_resuming_tail() {
    let actor = test_actor_inner();
    let polls = Arc::new((0..17).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    let mut items: VecDeque<TestFuture> = (0..17)
        .map(|index| {
            Box::pin(IndexedPoll {
                index,
                polls: Arc::clone(&polls),
                completes: index == 0,
            }) as TestFuture
        })
        .collect();
    items.rotate_left(2);
    let mut sweep = SweepState::default();
    let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::BudgetExhausted
    );
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Pending
    );
    assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) == 1));
}

// A panicking poll must leave its future owned.
#[test]
fn poll_panic_retains_the_future_for_contained_cleanup() {
    let actor = test_actor_inner();
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut items = VecDeque::from([Box::pin(PanicOnPoll {
        dropped: Arc::clone(&dropped),
        dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
    }) as TestFuture]);
    let mut sweep = SweepState::default();
    let mut task = Context::from_waker(Waker::noop());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task)
    }));
    assert!(result.is_err());
    assert!(!dropped.load(Ordering::SeqCst));

    drop(result);
    actor
        .control
        .drop_user_value(items.pop_front().expect("the failed future stays owned"));
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
}

#[test]
fn completed_future_drop_panic_is_contained_after_removal() {
    let actor = test_actor_inner();
    let drops = Arc::new(AtomicUsize::new(0));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let mut items = VecDeque::from([Box::pin(ReadyWithPanickingDrop {
        drops: Arc::clone(&drops),
        dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
    }) as TestFuture]);
    let mut sweep = SweepState::default();
    let mut task = Context::from_waker(Waker::noop());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task)
    }));

    assert!(matches!(result, Ok(InterleavedPoll::Progress)));
    assert!(items.is_empty());
    assert!(sweep.wake.is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(actor.control.mode(), Mode::Failing);
}

#[test]
fn kill_committed_by_one_reply_stops_the_sweep() {
    let actor = test_actor_inner();
    let first_polls = Arc::new(AtomicUsize::new(0));
    let second_polls = Arc::new(AtomicUsize::new(0));
    let mut items = VecDeque::from([
        Box::pin(KillOnPoll {
            actor: Arc::clone(&actor),
            polls: Arc::clone(&first_polls),
        }) as TestFuture,
        Box::pin(PollCounter(Arc::clone(&second_polls))) as TestFuture,
    ]);
    let mut sweep = SweepState::default();
    let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(wakes);
    let mut task = Context::from_waker(&waker);

    assert_eq!(
        poll_futures(&mut items, &mut sweep, &actor.control, &mut task),
        InterleavedPoll::Progress
    );
    assert_eq!(first_polls.load(Ordering::SeqCst), 1);
    assert_eq!(second_polls.load(Ordering::SeqCst), 0);
    assert_eq!(actor.control.mode(), Mode::Killing);
}
