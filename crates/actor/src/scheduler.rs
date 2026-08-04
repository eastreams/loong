use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use crate::{
    Actor, ActorFuture, ActorScope,
    mailbox::{Control, Mode},
};

const ACTIVE_POLL_BUDGET: usize = 16;

type ErasedActorFuture<A> = Pin<Box<dyn ActorFuture<A, Output = ()> + Send + 'static>>;

/// Outcome of one interleaved collection poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterleavedPoll {
    /// No reply completed, and no budget cutoff remains.
    Pending,
    /// A reply completed or lifecycle changed.
    Progress,
    /// The budget ended before the current sweep finished.
    ///
    /// The caller must stop polling ready lanes and return `Pending`.
    BudgetExhausted,
}

/// Polls replies that require temporary actor access.
pub(crate) struct ReplyScheduler<A: Actor> {
    interleaved: VecDeque<ErasedActorFuture<A>>,
    exclusive: Option<ErasedActorFuture<A>>,
    interleaved_sweep: SweepState,
    max_interleaved: NonZeroUsize,
}

impl<A: Actor> ReplyScheduler<A> {
    pub(crate) fn new(max_interleaved: NonZeroUsize) -> Self {
        Self {
            interleaved: VecDeque::new(),
            exclusive: None,
            interleaved_sweep: SweepState::default(),
            max_interleaved,
        }
    }

    pub(crate) fn has_dispatch_capacity(&self) -> bool {
        self.exclusive.is_none() && self.interleaved.len() < self.max_interleaved.get()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.interleaved.is_empty() && self.exclusive.is_none()
    }

    pub(crate) fn has_exclusive(&self) -> bool {
        self.exclusive.is_some()
    }

    pub(crate) fn has_interleaved(&self) -> bool {
        !self.interleaved.is_empty()
    }

    pub(crate) fn push_interleaved<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.interleaved.push_back(Box::pin(future));
        self.interleaved_sweep.restart();
        debug_assert!(self.interleaved.len() <= self.max_interleaved.get());
    }

    pub(crate) fn push_exclusive<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        debug_assert!(self.exclusive.is_none(), "exclusive work cannot overlap");
        self.exclusive = Some(Box::pin(future));
    }

    pub(crate) fn poll_interleaved(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> InterleavedPoll {
        poll_round_robin(
            &mut self.interleaved,
            &mut self.interleaved_sweep,
            control,
            expected_mode,
            task,
            |future, task| future.as_mut().poll(actor, scope, task),
        )
    }

    pub(crate) fn poll_exclusive(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        let Some(exclusive) = &mut self.exclusive else {
            return Poll::Pending;
        };
        let result = exclusive.as_mut().poll(actor, scope, task);
        // A final actor-aware poll may commit Stop or Drain. Retire completed
        // work before observing that mode change so graceful finalization cannot
        // poll the already-completed user future again.
        if result.is_ready() {
            self.exclusive = None;
        }
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        if result.is_pending() {
            return Poll::Pending;
        }

        Poll::Ready(())
    }

    /// Polls the eligible actor-aware lane while finishing replies.
    ///
    /// This method polls no mailbox or child work.
    /// Inactivity and budget exhaustion both return `Poll::Pending` here.
    /// Normal turns call `poll_interleaved` to preserve that distinction.
    /// `Poll::Ready` reports progress or a lifecycle change.
    pub(crate) fn poll_active(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        if self.has_exclusive() {
            self.poll_exclusive(actor, scope, control, expected_mode, task)
        } else {
            match self.poll_interleaved(actor, scope, control, expected_mode, task) {
                InterleavedPoll::Pending | InterleavedPoll::BudgetExhausted => Poll::Pending,
                InterleavedPoll::Progress => Poll::Ready(()),
            }
        }
    }

    /// Drops every retained reply through an independent panic boundary.
    ///
    /// One user destructor cannot skip another ownership slot.
    pub(crate) fn clear(&mut self, control: &Control) {
        if let Some(exclusive) = self.exclusive.take() {
            control.drop_user_value(exclusive);
        }
        for interleaved in self.interleaved.drain(..) {
            control.drop_user_value(interleaved);
        }
        self.interleaved_sweep.clear();
    }
}

/// Persistent recovery point for one collection's logical round-robin sweep.
#[derive(Default)]
struct SweepState {
    remaining: usize,
    generation: usize,
    wake: Arc<SweepWaker>,
}

impl SweepState {
    fn restart(&mut self) {
        self.remaining = 0;
    }

    fn clear(&mut self) {
        self.wake.clear();
        self.remaining = 0;
        self.generation = 0;
    }
}

#[derive(Default)]
struct SweepWaker {
    generation: AtomicUsize,
    task: Mutex<Option<Waker>>,
}

impl SweepWaker {
    fn register(&self, waker: &Waker) {
        let mut task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if task
            .as_ref()
            .is_none_or(|current| !current.will_wake(waker))
        {
            *task = Some(waker.clone());
        }
    }

    fn generation(&self) -> usize {
        self.generation.load(Ordering::Acquire)
    }

    fn clear(&self) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(task);
    }

    fn notify(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(task) = task {
            task.wake();
        }
    }
}

impl Wake for SweepWaker {
    fn wake(self: Arc<Self>) {
        self.notify();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify();
    }
}

// The deque front is the persistent round-robin cursor.
// Pending work rotates back. Completed work leaves from the front.
// An idle state starts one circular sweep.
// Each call polls at most ACTIVE_POLL_BUDGET items.
// A truncated sweep self-wakes and returns BudgetExhausted.
// The deque front and `remaining` preserve its recovery point.
// One user poll may still run without returning.
//
// Every item receives the same proxy waker.
// Its generation separates future notifications from continuation wakes.
// A coalesced notification starts one confirmation sweep.
// This prevents an all-pending collection from spinning.
//
// Progress reports completion or lifecycle change.
// Pending means no work completed and no budget cutoff remains.
fn poll_round_robin<T>(
    items: &mut VecDeque<T>,
    sweep: &mut SweepState,
    control: &Control,
    expected_mode: Mode,
    task: &mut Context<'_>,
    mut poll: impl FnMut(&mut T, &mut Context<'_>) -> Poll<()>,
) -> InterleavedPoll {
    if items.is_empty() {
        sweep.clear();
        return InterleavedPoll::Pending;
    }

    sweep.wake.register(task.waker());
    let item_waker = Waker::from(Arc::clone(&sweep.wake));
    let mut item_task = Context::from_waker(&item_waker);

    if sweep.remaining == 0 {
        sweep.remaining = items.len();
        sweep.generation = sweep.wake.generation();
    } else {
        sweep.remaining = sweep.remaining.min(items.len());
    }
    let poll_count = sweep.remaining.min(ACTIVE_POLL_BUDGET);
    let mut polled = 0;
    let mut completed = false;

    for _ in 0..poll_count {
        if control.mode() != expected_mode {
            break;
        }
        // Poll before removal. Panic cleanup must retain scheduler ownership.
        match poll(&mut items[0], &mut item_task) {
            Poll::Ready(()) => {
                let completed_item = items.pop_front().expect("the front item was just polled");
                drop(completed_item);
                completed = true;
            }
            Poll::Pending => items.rotate_left(1),
        }
        polled += 1;
    }

    sweep.remaining -= polled;
    if control.mode() != expected_mode {
        return InterleavedPoll::Progress;
    }
    if items.is_empty() {
        sweep.clear();
    } else if sweep.remaining > 0 {
        task.waker().wake_by_ref();
        return InterleavedPoll::BudgetExhausted;
    } else if sweep.wake.generation() != sweep.generation {
        task.waker().wake_by_ref();
    }

    if completed {
        InterleavedPoll::Progress
    } else {
        InterleavedPoll::Pending
    }
}

#[cfg(test)]
mod tests {
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
    use crate::mailbox::ActorInner;

    struct TestActor;

    #[crate::actor(mailbox = 1, interleaved = dynamic(2))]
    impl Actor for TestActor {
        type SpawnArgs = ();

        async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
            Self
        }
    }

    fn test_actor_inner() -> Arc<ActorInner<TestActor>> {
        let options = <TestActor as crate::ActorConfig>::Options::default();
        ActorInner::open(&options).0
    }

    struct DropProbe {
        dropped: Arc<AtomicBool>,
        dropped_while_unwinding: Arc<AtomicBool>,
        panic: bool,
    }

    impl ActorFuture<TestActor> for DropProbe {
        type Output = ();

        fn poll(
            self: Pin<&mut Self>,
            _actor: &mut TestActor,
            _scope: &mut ActorScope<'_, TestActor>,
            _task: &mut Context<'_>,
        ) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
            self.dropped_while_unwinding
                .store(std::thread::panicking(), Ordering::SeqCst);
            assert!(!self.panic, "intentional actor future drop panic");
        }
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

    impl Wake for NotifyFlag {
        fn wake(self: Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.store(true, Ordering::SeqCst);
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
            self.actor.control.request(crate::Shutdown::Kill);
            Poll::Pending
        }
    }

    type TestFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

    // Tests the shared sweep without constructing an actor scope.
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

    // The completion sits at the first budget's final position.
    // Progress would let run_actor consume the next budget immediately.
    // BudgetExhausted must yield while retaining the unpolled tail.
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

    // Polling must not move the future out of scheduler ownership.
    // Otherwise its destructor may panic during the poll unwind.
    // Containment must finish before later cleanup drops the future.
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
    fn kill_committed_by_one_reply_prevents_polling_the_next_reply() {
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

    // Exclusive Drop runs before the interleaved collection is cleared.
    // Each boundary must catch panic before the next destructor runs.
    // The two cases use one panic each to avoid double-panic aborts.
    #[test]
    fn clear_contains_each_actor_future_drop() {
        let actor = test_actor_inner();
        let exclusive_dropped = Arc::new(AtomicBool::new(false));
        let exclusive_tail_dropped = Arc::new(AtomicBool::new(false));
        let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
        let mut scheduler = ReplyScheduler::new(NonZeroUsize::MIN);
        scheduler.push_exclusive(DropProbe {
            dropped: Arc::clone(&exclusive_dropped),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: true,
        });
        scheduler.push_interleaved(DropProbe {
            dropped: Arc::clone(&exclusive_tail_dropped),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: false,
        });

        scheduler.clear(&actor.control);
        assert!(exclusive_dropped.load(Ordering::SeqCst));
        assert!(exclusive_tail_dropped.load(Ordering::SeqCst));
        assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
        assert!(scheduler.is_empty());
        assert_eq!(actor.control.mode(), Mode::Failing);

        let actor = test_actor_inner();
        let interleaved_panicked = Arc::new(AtomicBool::new(false));
        let tail_dropped = Arc::new(AtomicBool::new(false));
        let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(2).unwrap());
        scheduler.push_interleaved(DropProbe {
            dropped: Arc::clone(&interleaved_panicked),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: true,
        });
        scheduler.push_interleaved(DropProbe {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            panic: false,
        });

        scheduler.clear(&actor.control);
        assert!(interleaved_panicked.load(Ordering::SeqCst));
        assert!(tail_dropped.load(Ordering::SeqCst));
        assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
        assert!(scheduler.is_empty());
        assert_eq!(actor.control.mode(), Mode::Failing);
    }
}
