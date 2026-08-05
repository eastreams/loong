use std::{
    sync::Arc,
    task::{Context, Poll},
};

use crate::{
    Actor, ActorFuture, ChildExit,
    config::ReplySchedulingConfig,
    mailbox::{ActorInbox, ActorInner, Control, Mode},
    owned::OwnedTasks,
    runtime::ScopeState,
    transport::{MessageInbox, MessageSender},
};

use super::{
    Dynamic, Exclusive, Fixed, InterleavedLane, InterleavedProfile, Serial, SerialLane, Unbounded,
    queue::InterleavedPoll,
};

pub(crate) type ActorScheduler<A> = <A as ReplySchedulingConfig>::Scheduler;

/// Result of one scheduler-owned actor turn.
pub(crate) enum SchedulerTurn {
    /// Notification or lifecycle mismatch.
    LifecycleHint,
    /// Mailbox dispatch or an actor-aware reply made progress.
    Progress,
    /// One direct child terminated.
    Child(ChildExit),
    /// The mailbox receiver closed.
    InboxClosed,
}

/// Borrows actor-task resources for one scheduler poll.
pub(crate) struct TurnContext<'a, A: Actor> {
    pub(crate) actor: &'a mut A,
    pub(crate) state: &'a mut ScopeState<A>,
    pub(crate) inbox: &'a mut ActorInbox<A>,
    pub(crate) inner: &'a Arc<ActorInner<A>>,
    pub(crate) owned: &'a OwnedTasks<A>,
    pub(crate) receive_messages: bool,
    pub(crate) expected_mode: Mode,
}

pub(crate) trait RuntimeScheduler<A: Actor>: Send + 'static {
    fn is_idle(&mut self) -> bool;

    fn push_exclusive<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static;

    fn poll_actor_replies(
        &mut self,
        actor: &mut A,
        scope: &mut crate::ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()>;

    fn poll_turn(
        &mut self,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn>;

    fn clear(&mut self, control: &Control);
}

pub(crate) trait RuntimeInterleavedScheduler<A: Actor>: RuntimeScheduler<A> {
    fn push_interleaved<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static;
}

trait SchedulerProfile<A: Actor>: Send + 'static + Sized {
    type Strategy: SchedulerStrategy<A, Self>;
}

trait SchedulerStrategy<A: Actor, P: Send + 'static>: Send + 'static {
    fn is_idle(scheduler: &mut P) -> bool;

    fn push_exclusive<F>(scheduler: &mut P, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static;

    fn poll_actor_replies(
        scheduler: &mut P,
        actor: &mut A,
        scope: &mut crate::ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()>;

    fn poll_turn(
        scheduler: &mut P,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn>;

    fn clear(scheduler: &mut P, control: &Control);
}

struct SerialStrategy;
struct InterleavedStrategy;

impl<A> SchedulerProfile<A> for Serial<A>
where
    A: Actor + ReplySchedulingConfig<Scheduler = Self>,
{
    type Strategy = SerialStrategy;
}

impl<A, const N: usize> SchedulerProfile<A> for Fixed<A, N>
where
    A: Actor + ReplySchedulingConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = InterleavedStrategy;
}

impl<A> SchedulerProfile<A> for Dynamic<A>
where
    A: Actor + ReplySchedulingConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = InterleavedStrategy;
}

impl<A> SchedulerProfile<A> for Unbounded<A>
where
    A: Actor + ReplySchedulingConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = InterleavedStrategy;
}

// Coherence cannot treat InterleavedProfile as a closed set.
// This private strategy selects one of two monomorphized loops.
impl<A, P> RuntimeScheduler<A> for P
where
    A: Actor + ReplySchedulingConfig<Scheduler = P>,
    P: SchedulerProfile<A>,
{
    fn is_idle(&mut self) -> bool {
        P::Strategy::is_idle(self)
    }

    fn push_exclusive<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        P::Strategy::push_exclusive(self, future);
    }

    fn poll_actor_replies(
        &mut self,
        actor: &mut A,
        scope: &mut crate::ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        P::Strategy::poll_actor_replies(self, actor, scope, control, expected_mode, task)
    }

    fn poll_turn(
        &mut self,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn> {
        P::Strategy::poll_turn(self, turn, task)
    }

    fn clear(&mut self, control: &Control) {
        P::Strategy::clear(self, control);
    }
}

// Serial and interleaved profiles keep separate lane loops.
// Serial therefore acquires no interleaving protocol.
impl<A> SchedulerStrategy<A, Serial<A>> for SerialStrategy
where
    A: Actor + ReplySchedulingConfig<Scheduler = Serial<A>>,
{
    fn is_idle(scheduler: &mut Serial<A>) -> bool {
        scheduler.exclusive.is_empty()
    }

    fn push_exclusive<F>(scheduler: &mut Serial<A>, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        scheduler.exclusive.push(future);
    }

    fn poll_actor_replies(
        scheduler: &mut Serial<A>,
        actor: &mut A,
        scope: &mut crate::ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        scheduler
            .exclusive
            .poll(actor, scope, control, expected_mode, task)
    }

    fn poll_turn(
        scheduler: &mut Serial<A>,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn> {
        if turn.inner.control.mode() != turn.expected_mode {
            return Poll::Ready(SchedulerTurn::LifecycleHint);
        }

        if !scheduler.exclusive.is_empty() {
            return poll_exclusive_turn(&mut scheduler.exclusive, turn, task);
        }

        let start = scheduler.cursor;
        let mut lane = start;
        loop {
            if turn.inner.control.mode() != turn.expected_mode {
                return Poll::Ready(SchedulerTurn::LifecycleHint);
            }
            let selected = match lane {
                SerialLane::Mailbox if turn.receive_messages => {
                    let mut scope = turn.state.actor_scope();
                    let mut dispatched = 0;
                    loop {
                        match turn.inbox.poll_recv(task) {
                            Poll::Ready(Some(envelope)) => {
                                envelope.dispatch(
                                    turn.actor, &mut scope, turn.owned, scheduler, turn.inner,
                                );
                                dispatched += 1;
                            }
                            Poll::Ready(None) => break Some(SchedulerTurn::InboxClosed),
                            Poll::Pending => break None,
                        }

                        if turn.inner.control.mode() != turn.expected_mode {
                            scheduler.cursor = lane.next();
                            return Poll::Ready(SchedulerTurn::LifecycleHint);
                        }
                        if dispatched == A::MAILBOX_DISPATCH_BUDGET.get()
                            || !scheduler.exclusive.is_empty()
                        {
                            break Some(SchedulerTurn::Progress);
                        }
                    }
                }
                SerialLane::ChildExit => poll_child(turn.state, task),
                _ => None,
            };

            if let Some(turn) = selected {
                scheduler.cursor = lane.next();
                return Poll::Ready(turn);
            }

            lane = lane.next();
            if lane == start {
                break;
            }
        }

        scheduler.cursor = start.next();
        Poll::Pending
    }

    fn clear(scheduler: &mut Serial<A>, control: &Control) {
        scheduler.exclusive.clear(control);
    }
}

impl<A, P> SchedulerStrategy<A, P> for InterleavedStrategy
where
    A: Actor + ReplySchedulingConfig<Scheduler = P>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
    P: InterleavedProfile<A>,
{
    fn is_idle(scheduler: &mut P) -> bool {
        let state = scheduler.state();
        state.exclusive.is_empty() && !state.has_interleaved()
    }

    fn push_exclusive<F>(scheduler: &mut P, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        scheduler.state().exclusive.push(future);
    }

    fn poll_actor_replies(
        scheduler: &mut P,
        actor: &mut A,
        scope: &mut crate::ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        let state = scheduler.state();
        if !state.exclusive.is_empty() {
            state
                .exclusive
                .poll(actor, scope, control, expected_mode, task)
        } else {
            match state.queue.poll(actor, scope, control, expected_mode, task) {
                InterleavedPoll::Pending | InterleavedPoll::BudgetExhausted => Poll::Pending,
                InterleavedPoll::Progress => Poll::Ready(()),
            }
        }
    }

    fn poll_turn(
        scheduler: &mut P,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn> {
        if turn.inner.control.mode() != turn.expected_mode {
            return Poll::Ready(SchedulerTurn::LifecycleHint);
        }

        if !scheduler.state().exclusive.is_empty() {
            return poll_exclusive_turn(&mut scheduler.state().exclusive, turn, task);
        }

        let start = scheduler.state().cursor;
        let mut lane = start;
        loop {
            if turn.inner.control.mode() != turn.expected_mode {
                return Poll::Ready(SchedulerTurn::LifecycleHint);
            }
            let selected = match lane {
                InterleavedLane::Mailbox
                    if turn.receive_messages && scheduler.state().has_dispatch_capacity() =>
                {
                    let mut scope = turn.state.actor_scope();
                    let mut dispatched = 0;
                    loop {
                        match turn.inbox.poll_recv(task) {
                            Poll::Ready(Some(envelope)) => {
                                envelope.dispatch(
                                    turn.actor, &mut scope, turn.owned, scheduler, turn.inner,
                                );
                                dispatched += 1;
                            }
                            Poll::Ready(None) => break Some(SchedulerTurn::InboxClosed),
                            Poll::Pending
                                if dispatched > 0
                                    && start == InterleavedLane::Interleaved
                                    && scheduler.state().has_interleaved() =>
                            {
                                break Some(SchedulerTurn::Progress);
                            }
                            Poll::Pending => break None,
                        }

                        if turn.inner.control.mode() != turn.expected_mode {
                            scheduler.state().cursor = lane.next();
                            return Poll::Ready(SchedulerTurn::LifecycleHint);
                        }
                        if dispatched == A::MAILBOX_DISPATCH_BUDGET.get()
                            || !scheduler.state().has_dispatch_capacity()
                        {
                            break Some(SchedulerTurn::Progress);
                        }
                    }
                }
                InterleavedLane::Interleaved if scheduler.state().has_interleaved() => {
                    let mut scope = turn.state.actor_scope();
                    match scheduler.state().queue.poll(
                        turn.actor,
                        &mut scope,
                        &turn.inner.control,
                        turn.expected_mode,
                        task,
                    ) {
                        InterleavedPoll::Pending => None,
                        InterleavedPoll::Progress => Some(SchedulerTurn::Progress),
                        InterleavedPoll::BudgetExhausted => {
                            scheduler.state().cursor = lane.next();
                            return Poll::Pending;
                        }
                    }
                }
                InterleavedLane::ChildExit => poll_child(turn.state, task),
                _ => None,
            };

            if let Some(turn) = selected {
                scheduler.state().cursor = lane.next();
                return Poll::Ready(turn);
            }

            lane = lane.next();
            if lane == start {
                break;
            }
        }

        scheduler.state().cursor = start.next();
        Poll::Pending
    }

    fn clear(scheduler: &mut P, control: &Control) {
        let state = scheduler.state();
        state.exclusive.clear(control);
        state.queue.clear(control);
    }
}

#[diagnostic::do_not_recommend]
impl<A, P> RuntimeInterleavedScheduler<A> for P
where
    A: Actor + ReplySchedulingConfig<Scheduler = P>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
    P: InterleavedProfile<A>,
{
    fn push_interleaved<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state().push_interleaved(Box::pin(future));
    }
}

fn poll_exclusive_turn<A: Actor>(
    exclusive: &mut Exclusive<A>,
    turn: &mut TurnContext<'_, A>,
    task: &mut Context<'_>,
) -> Poll<SchedulerTurn> {
    let mut scope = turn.state.actor_scope();
    let ready = exclusive
        .poll(
            turn.actor,
            &mut scope,
            &turn.inner.control,
            turn.expected_mode,
            task,
        )
        .is_ready();
    if turn.inner.control.mode() != turn.expected_mode {
        return Poll::Ready(SchedulerTurn::LifecycleHint);
    }
    if ready {
        Poll::Ready(SchedulerTurn::Progress)
    } else {
        Poll::Pending
    }
}

fn poll_child<A: Actor>(
    state: &mut ScopeState<A>,
    task: &mut Context<'_>,
) -> Option<SchedulerTurn> {
    match state.poll_child_exit(task) {
        Poll::Ready(event) => Some(SchedulerTurn::Child(event)),
        Poll::Pending => None,
    }
}

#[cfg(test)]
mod tests;
