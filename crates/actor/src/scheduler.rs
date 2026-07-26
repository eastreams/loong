use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use crate::{
    Actor, ActorFuture, ActorScope, ErasedFuture,
    mailbox::{Control, Mode},
};

const ACTIVE_POLL_BUDGET: usize = 16;

type ErasedActorFuture<A> = Pin<Box<dyn ActorFuture<A, Output = ()> + Send + 'static>>;

/// Actor-local ownership boundary for replies that outlive handler dispatch.
/// Its collections cannot exceed `max_in_flight`; no stored future borrows the
/// actor or scope between polls.
pub(crate) struct ReplyScheduler<A: Actor> {
    owned: Vec<ErasedFuture<'static>>,
    interleaved: Vec<ErasedActorFuture<A>>,
    exclusive: Option<ErasedActorFuture<A>>,
    owned_sweep: SweepState,
    interleaved_sweep: SweepState,
    max_in_flight: NonZeroUsize,
}

impl<A: Actor> ReplyScheduler<A> {
    pub(crate) fn new(max_in_flight: NonZeroUsize) -> Self {
        Self {
            owned: Vec::new(),
            interleaved: Vec::new(),
            exclusive: None,
            owned_sweep: SweepState::default(),
            interleaved_sweep: SweepState::default(),
            max_in_flight,
        }
    }

    pub(crate) fn can_dispatch(&self) -> bool {
        self.in_flight() < self.max_in_flight.get()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.in_flight() == 0
    }

    pub(crate) fn has_exclusive(&self) -> bool {
        self.exclusive.is_some()
    }

    pub(crate) fn has_owned(&self) -> bool {
        !self.owned.is_empty()
    }

    pub(crate) fn has_interleaved(&self) -> bool {
        !self.interleaved.is_empty()
    }

    pub(crate) fn push_owned<F>(&mut self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.owned.push(Box::pin(future));
        self.owned_sweep.restart();
        debug_assert!(self.in_flight() <= self.max_in_flight.get());
    }

    pub(crate) fn push_interleaved<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.interleaved.push(Box::pin(future));
        self.interleaved_sweep.restart();
        debug_assert!(self.in_flight() <= self.max_in_flight.get());
    }

    pub(crate) fn push_exclusive<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        debug_assert!(self.exclusive.is_none(), "exclusive work cannot overlap");
        self.exclusive = Some(Box::pin(future));
        debug_assert!(self.in_flight() <= self.max_in_flight.get());
    }

    pub(crate) fn poll_owned(
        &mut self,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        poll_collection(
            &mut self.owned,
            &mut self.owned_sweep,
            control,
            expected_mode,
            task,
            |future, task| future.as_mut().poll(task).is_ready(),
        )
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
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        if result.is_pending() {
            return Poll::Pending;
        }

        self.exclusive = None;
        Poll::Ready(())
    }

    pub(crate) fn poll_active(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        let mut completed = self.poll_owned(control, expected_mode, task).is_ready();
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        completed |= if self.has_exclusive() {
            self.poll_exclusive(actor, scope, control, expected_mode, task)
                .is_ready()
        } else {
            self.poll_interleaved(actor, scope, control, expected_mode, task)
                .is_ready()
        };
        if completed {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    pub(crate) fn clear(&mut self) {
        self.exclusive = None;
        self.interleaved.clear();
        self.owned.clear();
        self.interleaved_sweep.clear();
        self.owned_sweep.clear();
    }

    fn in_flight(&self) -> usize {
        self.owned.len() + self.interleaved.len() + usize::from(self.exclusive.is_some())
    }
}

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

// Each collection gets one proxy waker. Its generation distinguishes future
// notifications from budget continuation wakes, so a coalesced external wake
// triggers a confirmation sweep without making an all-pending collection spin.
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

    #[test]
    fn budgeted_owned_scan_resumes_at_the_unpolled_tail() {
        let mut scheduler = ReplyScheduler::<TestActor>::new(NonZeroUsize::new(32).unwrap());
        let control = Control::new();
        let polls: Vec<_> = (0..20).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        for count in &polls {
            scheduler.push_owned(PollCounter(Arc::clone(count)));
        }

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut task = Context::from_waker(&waker);

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
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

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
        );
        assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) > 0));
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn future_wake_coalesced_with_continuation_starts_another_sweep() {
        let mut scheduler = ReplyScheduler::<TestActor>::new(NonZeroUsize::new(32).unwrap());
        let control = Control::new();
        let first_polls = Arc::new(AtomicUsize::new(0));
        let first_waker = Arc::new(Mutex::new(None));
        scheduler.push_owned(CaptureWaker {
            polls: Arc::clone(&first_polls),
            waker: Arc::clone(&first_waker),
        });
        for _ in 1..20 {
            scheduler.push_owned(std::future::pending());
        }

        let notified = Arc::new(NotifyFlag(AtomicBool::new(false)));
        let waker = Waker::from(Arc::clone(&notified));
        let mut task = Context::from_waker(&waker);

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
        );
        first_waker.lock().unwrap().as_ref().unwrap().wake_by_ref();
        assert!(notified.0.swap(false, Ordering::SeqCst));

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
        );
        assert!(notified.0.swap(false, Ordering::SeqCst));

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
        );
        assert_eq!(first_polls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn completion_does_not_skip_the_unpolled_item_after_cursor_wraps() {
        let mut scheduler = ReplyScheduler::<TestActor>::new(NonZeroUsize::new(32).unwrap());
        let control = Control::new();
        let polls = Arc::new((0..17).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        for index in 0..17 {
            scheduler.push_owned(IndexedPoll {
                index,
                polls: Arc::clone(&polls),
                completes: index == 0,
            });
        }
        scheduler.owned_sweep.cursor = 2;

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut task = Context::from_waker(&waker);

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_ready()
        );
        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_pending()
        );
        assert!(polls.iter().all(|count| count.load(Ordering::SeqCst) == 1));
    }

    #[test]
    fn kill_committed_by_one_reply_prevents_polling_the_next_reply() {
        let mut scheduler = ReplyScheduler::<TestActor>::new(NonZeroUsize::new(2).unwrap());
        let control = Control::new();
        let first_polls = Arc::new(AtomicUsize::new(0));
        let second_polls = Arc::new(AtomicUsize::new(0));
        scheduler.push_owned(KillOnPoll {
            control: Arc::clone(&control),
            polls: Arc::clone(&first_polls),
        });
        scheduler.push_owned(PollCounter(Arc::clone(&second_polls)));

        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(wakes);
        let mut task = Context::from_waker(&waker);

        assert!(
            scheduler
                .poll_owned(&control, Mode::Running, &mut task)
                .is_ready()
        );
        assert_eq!(first_polls.load(Ordering::SeqCst), 1);
        assert_eq!(second_polls.load(Ordering::SeqCst), 0);
        assert_eq!(control.mode(), Mode::Killing);
    }

    struct TestActor;

    impl Actor for TestActor {}
}
