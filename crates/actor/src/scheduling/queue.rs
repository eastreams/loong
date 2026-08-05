use std::{
    collections::VecDeque,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use futures_util::task::AtomicWaker;

use crate::{
    Actor, ActorScope,
    mailbox::{Control, Mode},
};

use super::{ErasedActorFuture, drop_without_unwind};

const ACTIVE_POLL_BUDGET: usize = 16;

/// Outcome of one interleaved collection poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InterleavedPoll {
    /// No reply completed, and no budget cutoff remains.
    Pending,
    /// A reply completed or lifecycle changed.
    Progress,
    /// The budget ended before the current sweep finished.
    ///
    /// The caller must stop polling ready lanes and return `Pending`.
    BudgetExhausted,
}

pub(super) struct Queue<A: Actor> {
    items: VecDeque<ErasedActorFuture<A>>,
    sweep: SweepState,
}

impl<A: Actor> Queue<A> {
    pub(super) fn new() -> Self {
        Self {
            items: VecDeque::new(),
            sweep: SweepState::default(),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(super) fn push(&mut self, future: ErasedActorFuture<A>) {
        self.items.push_back(future);
        self.sweep.restart();
    }

    pub(super) fn poll(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> InterleavedPoll {
        poll_round_robin(
            &mut self.items,
            &mut self.sweep,
            control,
            expected_mode,
            task,
            |future, task| future.as_mut().poll(actor, scope, task),
        )
    }

    pub(super) fn clear(&mut self, control: &Control) {
        for future in self.items.drain(..) {
            control.drop_user_value(future);
        }
        self.sweep.clear();
    }
}

impl<A: Actor> Drop for Queue<A> {
    fn drop(&mut self) {
        self.sweep.clear();
        while let Some(future) = self.items.pop_front() {
            drop_without_unwind(future);
        }
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

    let wake = sweep
        .wake
        .get_or_insert_with(|| Arc::new(SweepWaker::default()));
    wake.register(task.waker());
    let item_waker = Waker::from(Arc::clone(wake));
    let mut item_task = Context::from_waker(&item_waker);

    if sweep.remaining == 0 {
        sweep.remaining = items.len();
        sweep.generation = wake.generation();
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
                control.drop_user_value(completed_item);
                completed = true;
            }
            Poll::Pending => items.rotate_left(1),
        }
        polled += 1;
    }

    sweep.remaining -= polled;
    if items.is_empty() {
        sweep.clear();
    }
    if control.mode() != expected_mode {
        return InterleavedPoll::Progress;
    }
    if sweep.remaining > 0 {
        sweep
            .wake
            .as_ref()
            .expect("an active sweep retains its waker")
            .wake_task();
        return InterleavedPoll::BudgetExhausted;
    } else if let Some(wake) = &sweep.wake
        && wake.generation() != sweep.generation
    {
        wake.wake_task();
    }

    if completed {
        InterleavedPoll::Progress
    } else {
        InterleavedPoll::Pending
    }
}

#[derive(Default)]
struct SweepState {
    remaining: usize,
    generation: usize,
    wake: Option<Arc<SweepWaker>>,
}

impl SweepState {
    fn restart(&mut self) {
        self.remaining = 0;
    }

    fn clear(&mut self) {
        if let Some(wake) = &self.wake {
            wake.clear();
        }
        self.wake = None;
        self.remaining = 0;
        self.generation = 0;
    }
}

#[derive(Default)]
struct SweepWaker {
    generation: AtomicUsize,
    task: AtomicWaker,
}

impl SweepWaker {
    fn register(&self, waker: &Waker) {
        self.task.register(waker);
    }

    fn generation(&self) -> usize {
        self.generation.load(Ordering::Acquire)
    }

    // A reactor may retain an item waker after its future is dropped.
    // Detach the actor task before releasing the scheduler's strong edge.
    fn clear(&self) {
        if let Some(task) = self.task.take() {
            drop_without_unwind(task);
        }
    }

    fn notify(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        self.wake_task();
    }

    // Continuation wakes must not create a future-notification generation.
    fn wake_task(&self) {
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| self.task.wake())) {
            Control::discard_panic(payload);
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

impl Drop for SweepWaker {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests;
