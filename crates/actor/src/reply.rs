//! Explicit reply scheduling constructors.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{
    Actor, ActorFuture, ActorScope, Message, mailbox::DispatchReply, scheduler::ReplyScheduler,
};

/// Creates an immediately completed reply.
pub fn ready<R>(value: R) -> Ready<R> {
    Ready { value }
}

/// Creates an actor-independent asynchronous reply.
pub fn owned<F>(future: F) -> Owned<F> {
    Owned { future }
}

/// Creates an actor-aware reply that may interleave with other actor work.
pub fn interleaved<F>(future: F) -> Interleaved<F> {
    Interleaved { future }
}

/// Creates an actor-aware reply with exclusive access between its polls.
pub fn exclusive<F>(future: F) -> Exclusive<F> {
    Exclusive { future }
}

/// An immediately completed reply created by [`ready`].
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Ready<R> {
    value: R,
}

/// An actor-independent reply created by [`owned`].
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Owned<F> {
    future: F,
}

/// An interleaved actor-aware reply created by [`interleaved`].
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Interleaved<F> {
    future: F,
}

/// An exclusive actor-aware reply created by [`exclusive`].
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Exclusive<F> {
    future: F,
}

/// One of two statically known reply strategies.
///
/// `Either` lets a handler choose a concrete scheduling mode at runtime without
/// boxing at the public handler boundary. More branches can be represented by
/// nesting `Either` values.
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub enum Either<L, R> {
    /// Uses the left reply strategy.
    Left(L),
    /// Uses the right reply strategy.
    Right(R),
}

/// A crate-controlled reply strategy returned by a handler.
///
/// This trait is sealed so reply senders and lifecycle error construction stay
/// private to the runtime. Use this trait as an opaque handler return bound and
/// construct values with [`ready`], [`owned`], [`interleaved`], or [`exclusive`].
pub trait IntoReply<A: Actor, M: Message>: sealed::HandleReply<A, M> {}

impl<A, M, T> IntoReply<A, M> for T
where
    A: Actor,
    M: Message,
    T: sealed::HandleReply<A, M>,
{
}

// Crate visibility lets the mailbox invoke the static reply implementation
// after dynamic envelope dispatch, while downstream crates cannot name it.
#[expect(
    private_interfaces,
    reason = "sealed reply dispatch deliberately uses crate-private runtime types"
)]
pub(crate) mod sealed {
    use super::*;

    pub trait HandleReply<A: Actor, M: Message> {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>);
    }

    impl<A, M> HandleReply<A, M> for Ready<M::Reply>
    where
        A: Actor,
        M: Message,
    {
        fn handle(self, _scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            reply.complete(self.value);
        }
    }

    impl<A, M, F> HandleReply<A, M> for Owned<F>
    where
        A: Actor,
        M: Message,
        F: Future<Output = M::Reply> + Send + 'static,
    {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            scheduler.push_owned(CompleteOwnedReply::new(self.future, reply));
        }
    }

    impl<A, M, F> HandleReply<A, M> for Interleaved<F>
    where
        A: Actor,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            scheduler.push_interleaved(CompleteReply::new(self.future, reply));
        }
    }

    impl<A, M, F> HandleReply<A, M> for Exclusive<F>
    where
        A: Actor,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            scheduler.push_exclusive(CompleteReply::new(self.future, reply));
        }
    }

    impl<A, M, L, R> HandleReply<A, M> for Either<L, R>
    where
        A: Actor,
        M: Message,
        L: HandleReply<A, M>,
        R: HandleReply<A, M>,
    {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            match self {
                Either::Left(left) => left.handle(scheduler, reply),
                Either::Right(right) => right.handle(scheduler, reply),
            }
        }
    }
}

pin_project! {
    struct CompleteOwnedReply<F, R> {
        // The guard is declared first so cancellation reports the lifecycle
        // error before running an arbitrary user future's Drop implementation.
        reply: Option<DispatchReply<R>>,
        #[pin]
        future: F,
    }
}

impl<F, R> CompleteOwnedReply<F, R> {
    fn new(future: F, reply: DispatchReply<R>) -> Self {
        Self {
            reply: Some(reply),
            future,
        }
    }
}

impl<F, R> Future for CompleteOwnedReply<F, R>
where
    F: Future<Output = R>,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let value = std::task::ready!(this.future.poll(task));
        this.reply
            .take()
            .expect("reply completion runs exactly once")
            .complete(value);
        Poll::Ready(())
    }
}

pin_project! {
    struct CompleteReply<F, R> {
        // See CompleteOwnedReply: lifecycle publication precedes user Drop.
        reply: Option<DispatchReply<R>>,
        #[pin]
        future: F,
    }
}

impl<F, R> CompleteReply<F, R> {
    fn new(future: F, reply: DispatchReply<R>) -> Self {
        Self {
            reply: Some(reply),
            future,
        }
    }
}

impl<A, F, R> ActorFuture<A> for CompleteReply<F, R>
where
    A: Actor,
    F: ActorFuture<A, Output = R>,
    R: Send + 'static,
{
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.project();
        let value = std::task::ready!(this.future.poll(actor, scope, task));
        this.reply
            .take()
            .expect("reply completion runs exactly once")
            .complete(value);
        Poll::Ready(())
    }
}
