#![allow(
    private_interfaces,
    reason = "public strategy proofs remain hidden by the private runtime module"
)]

use std::{
    sync::Arc,
    task::{Context, Poll},
};

use crate::{
    Actor, ChildExit,
    mailbox::{ActorInbox, ActorInner, Control, Mode},
    owned::OwnedTasks,
    runtime::ScopeState,
    transport::{MessageConfig, MessageInbox, MessageSender, NoInbox, NoSender},
};

use super::{
    Disabled, Exclusive, InterleavedLane, InterleavedProfile, SchedulerProfile, Serial, SerialLane,
    queue::InterleavedPoll,
};

pub(crate) type ActorScheduler<A> = <A as MessageConfig>::Scheduler;

/// Grants access to sealed reply runtime bridges.
pub struct Seal;

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

pub trait SchedulerStrategy<A: Actor, P: Send + 'static>: Send + 'static {
    fn is_idle(scheduler: &mut P) -> bool;

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

pub struct DisabledStrategy;
pub struct SerialStrategy;
pub struct InterleavedStrategy;

// One associated strategy selects a monomorphized actor loop.
impl<A, P> RuntimeScheduler<A> for P
where
    A: Actor + MessageConfig<Scheduler = P>,
    P: SchedulerProfile<A>,
{
    fn is_idle(&mut self) -> bool {
        P::Strategy::is_idle(self)
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

// Disabled has no mailbox or reply lanes.
// Child exits still need a supervision lane.
impl<A> SchedulerStrategy<A, Disabled> for DisabledStrategy
where
    A: Actor + MessageConfig<Sender = NoSender, Inbox = NoInbox, Scheduler = Disabled>,
{
    fn is_idle(_scheduler: &mut Disabled) -> bool {
        true
    }

    fn poll_actor_replies(
        _scheduler: &mut Disabled,
        _actor: &mut A,
        _scope: &mut crate::ActorScope<'_, A>,
        _control: &Control,
        _expected_mode: Mode,
        _task: &mut Context<'_>,
    ) -> Poll<()> {
        Poll::Pending
    }

    fn poll_turn(
        _scheduler: &mut Disabled,
        turn: &mut TurnContext<'_, A>,
        task: &mut Context<'_>,
    ) -> Poll<SchedulerTurn> {
        if turn.inner.control.mode() != turn.expected_mode {
            return Poll::Ready(SchedulerTurn::LifecycleHint);
        }

        match poll_child(turn.state, task) {
            Some(turn) => Poll::Ready(turn),
            None => Poll::Pending,
        }
    }

    fn clear(_scheduler: &mut Disabled, _control: &Control) {}
}

// Serial and interleaved profiles keep separate lane loops.
// Serial therefore acquires no interleaving protocol.
impl<A> SchedulerStrategy<A, Serial<A>> for SerialStrategy
where
    A: Actor + MessageConfig<Scheduler = Serial<A>>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn is_idle(scheduler: &mut Serial<A>) -> bool {
        scheduler.exclusive.is_empty()
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
    A: Actor + MessageConfig<Scheduler = P>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
    P: InterleavedProfile<A>,
{
    fn is_idle(scheduler: &mut P) -> bool {
        let state = scheduler.state();
        state.exclusive.is_empty() && !state.has_interleaved()
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
