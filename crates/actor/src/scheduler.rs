use std::{
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

/// Polls replies that require temporary actor access.
pub(crate) struct ReplyScheduler<A: Actor> {
    interleaved: Vec<ErasedActorFuture<A>>,
    exclusive: Option<ErasedActorFuture<A>>,
    interleaved_sweep: SweepState,
    max_interleaved: NonZeroUsize,
}

impl<A: Actor> ReplyScheduler<A> {
    pub(crate) fn new(max_interleaved: NonZeroUsize) -> Self {
        Self {
            interleaved: Vec::new(),
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
        self.interleaved.push(Box::pin(future));
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
        scope: &mut ActorScope<A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        poll_collection(
            &mut self.interleaved,
            &mut self.interleaved_sweep,
            control,
            expected_mode,
            task,
            |future, task| future.as_mut().poll(actor, scope, task).is_ready(),
        )
    }

    pub(crate) fn poll_exclusive(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<A>,
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

    /// Polls the eligible actor-aware lane.
    ///
    /// Ready reports reply progress or a lifecycle change, not that every active
    /// reply has completed.
    pub(crate) fn poll_active(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        if self.has_exclusive() {
            self.poll_exclusive(actor, scope, control, expected_mode, task)
        } else {
            self.poll_interleaved(actor, scope, control, expected_mode, task)
        }
    }

    pub(crate) fn clear(&mut self) {
        self.exclusive = None;
        self.interleaved.clear();
        self.interleaved_sweep.clear();
    }
}

/// Persistent recovery point for one collection's logical round-robin sweep.
#[derive(Default)]
struct SweepState {
    cursor: usize,
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
        self.cursor = 0;
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

// When no sweep is in progress, polling starts one logical circular pass over
// the eligible collection. The per-visit budget bounds how many items are polled
// before returning to other actor work; it cannot bound the
// duration of an individual user poll. `remaining` and `cursor` preserve the
// recovery point and, while the collection remains eligible, self-wake until the
// sweep is complete. Each collection gets one proxy waker whose generation
// distinguishes future notifications from budget continuation wakes, so a
// coalesced external wake triggers a confirmation sweep without making an
// all-pending collection spin. Ready means an item completed or lifecycle
// changed, not that the collection is empty.
fn poll_collection<T>(
    items: &mut Vec<T>,
    sweep: &mut SweepState,
    control: &Control,
    expected_mode: Mode,
    task: &mut Context<'_>,
    mut poll: impl FnMut(&mut T, &mut Context<'_>) -> bool,
) -> Poll<()> {
    if items.is_empty() {
        sweep.clear();
        return Poll::Pending;
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
        if sweep.cursor >= items.len() {
            sweep.cursor = 0;
        }
        if poll(&mut items[sweep.cursor], &mut item_task) {
            // Preserve the circular scan order. `swap_remove` can move an
            // already-polled tail item onto the cursor and leave the one
            // remaining unpolled item asleep after the sweep finishes.
            items.remove(sweep.cursor);
            completed = true;
            if sweep.cursor >= items.len() {
                sweep.cursor = 0;
            }
        } else {
            sweep.cursor = (sweep.cursor + 1) % items.len();
        }
        polled += 1;
    }

    sweep.remaining -= polled;
    if control.mode() != expected_mode {
        return Poll::Ready(());
    }
    if items.is_empty() {
        sweep.clear();
    } else if sweep.remaining > 0 || sweep.wake.generation() != sweep.generation {
        task.waker().wake_by_ref();
    }

    if completed {
        Poll::Ready(())
    } else {
        Poll::Pending
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

    struct KillOnPoll {
        control: Arc<Control>,
        polls: Arc<AtomicUsize>,
    }

    impl Future for KillOnPoll {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.control.request(crate::Shutdown::Kill);
            Poll::Pending
        }
    }

    type TestFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

    // Tests the shared sweep without constructing an actor scope.
    fn poll_futures(
        items: &mut Vec<TestFuture>,
        sweep: &mut SweepState,
        control: &Control,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        poll_collection(
            items,
            sweep,
            control,
            Mode::Running,
            task,
            |future, task| future.as_mut().poll(task).is_ready(),
        )
    }

    #[test]
    fn budgeted_scan_resumes_at_the_unpolled_tail() {
        let control = Control::new();
        let polls: Vec<_> = (0..20).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        let mut items: Vec<TestFuture> = polls
            .iter()
            .map(|count| Box::pin(PollCounter(Arc::clone(count))) as TestFuture)
            .collect();
        let mut sweep = SweepState::default();

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut task = Context::from_waker(&waker);

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
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

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
        assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) > 0));
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn future_wake_coalesced_with_continuation_starts_another_sweep() {
        let control = Control::new();
        let first_polls = Arc::new(AtomicUsize::new(0));
        let first_waker = Arc::new(Mutex::new(None));
        let mut items: Vec<TestFuture> = vec![Box::pin(CaptureWaker {
            polls: Arc::clone(&first_polls),
            waker: Arc::clone(&first_waker),
        })];
        for _ in 1..20 {
            items.push(Box::pin(std::future::pending()));
        }
        let mut sweep = SweepState::default();

        let notified = Arc::new(NotifyFlag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&notified));
        let mut task = Context::from_waker(&waker);

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
        first_waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
        assert!(notified.0.swap(false, Ordering::SeqCst));

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
        assert!(notified.0.swap(false, Ordering::SeqCst));

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
        assert_eq!(first_polls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn completion_does_not_skip_the_unpolled_item_after_cursor_wraps() {
        let control = Control::new();
        let polls = Arc::new((0..17).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let mut items: Vec<TestFuture> = (0..17)
            .map(|index| {
                Box::pin(IndexedPoll {
                    index,
                    polls: Arc::clone(&polls),
                    completes: index == 0,
                }) as TestFuture
            })
            .collect();
        let mut sweep = SweepState {
            cursor: 2,
            ..SweepState::default()
        };

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut task = Context::from_waker(&waker);

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_ready());
        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_pending());
        assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) == 1));
    }

    #[test]
    fn kill_committed_by_one_reply_prevents_polling_the_next_reply() {
        let control = Control::new();
        let first_polls = Arc::new(AtomicUsize::new(0));
        let second_polls = Arc::new(AtomicUsize::new(0));
        let mut items: Vec<TestFuture> = vec![
            Box::pin(KillOnPoll {
                control: Arc::clone(&control),
                polls: Arc::clone(&first_polls),
            }),
            Box::pin(PollCounter(Arc::clone(&second_polls))),
        ];
        let mut sweep = SweepState::default();

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(wakes);
        let mut task = Context::from_waker(&waker);

        assert!(poll_futures(&mut items, &mut sweep, &control, &mut task).is_ready());
        assert_eq!(first_polls.load(Ordering::SeqCst), 1);
        assert_eq!(second_polls.load(Ordering::SeqCst), 0);
        assert_eq!(control.mode(), Mode::Killing);
    }
}
