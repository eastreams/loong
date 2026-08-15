use std::{
    num::NonZeroUsize,
    task::{Context, Poll},
};

use crate::{
    Actor, ActorFuture, ActorScope,
    mailbox::{Control, Mode},
};

use super::{Dynamic, ErasedActorFuture, Fixed, Unbounded, drop_without_unwind, queue::Queue};

pub(crate) struct InterleavedState<A: Actor, L> {
    pub(super) queue: Queue<A>,
    pub(super) exclusive: Exclusive<A>,
    pub(super) limit: L,
    pub(crate) cursor: InterleavedLane,
}

pub(crate) trait InterleavedProfile<A: Actor>: Send + 'static {
    type Limit: LimitPolicy;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit>;
}

impl<A: Actor, const N: usize> InterleavedProfile<A> for Fixed<A, N> {
    type Limit = FixedLimit<N>;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

impl<A: Actor> InterleavedProfile<A> for Dynamic<A> {
    type Limit = DynamicLimit;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

impl<A: Actor> InterleavedProfile<A> for Unbounded<A> {
    type Limit = UnboundedLimit;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

pub(crate) struct FixedLimit<const N: usize>;

pub(crate) struct DynamicLimit(pub(super) NonZeroUsize);

pub(crate) struct UnboundedLimit;

pub(crate) trait LimitPolicy: Send + 'static {
    fn has_capacity(&self, active: usize) -> bool;
}

impl<const N: usize> LimitPolicy for FixedLimit<N> {
    fn has_capacity(&self, active: usize) -> bool {
        active < N
    }
}

impl LimitPolicy for DynamicLimit {
    fn has_capacity(&self, active: usize) -> bool {
        active < self.0.get()
    }
}

impl LimitPolicy for UnboundedLimit {
    fn has_capacity(&self, _active: usize) -> bool {
        true
    }
}

impl<A: Actor, L: LimitPolicy> InterleavedState<A, L> {
    pub(super) fn with_limit(limit: L) -> Self {
        Self {
            queue: Queue::new(),
            exclusive: Exclusive::new(),
            limit,
            cursor: InterleavedLane::Mailbox,
        }
    }

    pub(crate) fn has_dispatch_capacity(&self) -> bool {
        self.exclusive.is_empty() && self.limit.has_capacity(self.queue.len())
    }

    pub(crate) fn has_interleaved(&self) -> bool {
        !self.queue.is_empty()
    }

    pub(super) fn push_interleaved(&mut self, future: ErasedActorFuture<A>) {
        debug_assert!(self.has_dispatch_capacity());
        self.queue.push(future);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SerialLane {
    Mailbox,
    ChildExit,
}

impl SerialLane {
    pub(super) const fn next(self) -> Self {
        match self {
            Self::Mailbox => Self::ChildExit,
            Self::ChildExit => Self::Mailbox,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterleavedLane {
    Mailbox,
    Interleaved,
    ChildExit,
}

impl InterleavedLane {
    pub(super) const fn next(self) -> Self {
        match self {
            Self::Mailbox => Self::Interleaved,
            Self::Interleaved => Self::ChildExit,
            Self::ChildExit => Self::Mailbox,
        }
    }
}

pub(crate) struct Exclusive<A: Actor> {
    future: Option<ErasedActorFuture<A>>,
}

impl<A: Actor> Exclusive<A> {
    pub(super) fn new() -> Self {
        Self { future: None }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.future.is_none()
    }

    pub(super) fn push<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        debug_assert!(self.future.is_none(), "exclusive work cannot overlap");
        self.future = Some(Box::pin(future));
    }

    pub(super) fn poll(
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
        let Some(future) = &mut self.future else {
            return Poll::Pending;
        };
        let result = future.as_mut().poll(actor, scope, task);
        if result.is_ready() {
            let completed = self
                .future
                .take()
                .expect("the completed exclusive future remains owned");
            control.drop_user_value(completed);
        }
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        result
    }

    pub(super) fn clear(&mut self, control: &Control) {
        if let Some(future) = self.future.take() {
            control.drop_user_value(future);
        }
    }
}

impl<A: Actor> Drop for Exclusive<A> {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            drop_without_unwind(future);
        }
    }
}
