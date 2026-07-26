//! Explicit reply scheduling constructors.

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{Actor, ActorFuture, ActorScope, ErasedFuture, Message, mailbox::DispatchReply};

pub(crate) type ErasedActorFuture<A> = Pin<Box<dyn ActorFuture<A, Output = ()> + Send + 'static>>;

pub(crate) enum ReplyWork<A: Actor> {
    Complete,
    Owned(ErasedFuture<'static>),
    Interleaved(ErasedActorFuture<A>),
    Exclusive(ErasedActorFuture<A>),
}

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

/// A crate-controlled conversion from a handler result to scheduled work.
///
/// This trait is sealed so reply senders and lifecycle error construction stay
/// private to the runtime. Use this trait as an opaque handler return bound and
/// construct values with [`ready`], [`owned`], [`interleaved`], or [`exclusive`].
pub trait IntoReply<A: Actor, M: Message>: sealed::IntoReply<A, M> {}

impl<A, M, T> IntoReply<A, M> for T
where
    A: Actor,
    M: Message,
    T: sealed::IntoReply<A, M>,
{
}

// A public supertrait inside a private module is the standard sealing pattern.
// Its method deliberately mentions crate-private scheduling types so external
// code cannot invoke the conversion even through generic trait bounds.
#[allow(private_interfaces)]
mod sealed {
    use super::*;

    pub trait IntoReply<A: Actor, M: Message> {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A>;
    }

    impl<A, M> IntoReply<A, M> for Ready<M::Reply>
    where
        A: Actor,
        M: Message,
    {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A> {
            reply.complete(self.value);
            ReplyWork::Complete
        }
    }

    impl<A, M, F> IntoReply<A, M> for Owned<F>
    where
        A: Actor,
        M: Message,
        F: Future<Output = M::Reply> + Send + 'static,
    {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A> {
            ReplyWork::Owned(Box::pin(CompleteOwnedReply::new(self.future, reply)))
        }
    }

    impl<A, M, F> IntoReply<A, M> for Interleaved<F>
    where
        A: Actor,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A> {
            ReplyWork::Interleaved(Box::pin(CompleteReply::new(self.future, reply)))
        }
    }

    impl<A, M, F> IntoReply<A, M> for Exclusive<F>
    where
        A: Actor,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A> {
            ReplyWork::Exclusive(Box::pin(CompleteReply::new(self.future, reply)))
        }
    }

    impl<A, M, L, R> IntoReply<A, M> for Either<L, R>
    where
        A: Actor,
        M: Message,
        L: IntoReply<A, M>,
        R: IntoReply<A, M>,
    {
        fn into_reply(self, reply: DispatchReply<M::Reply>) -> ReplyWork<A> {
            match self {
                Either::Left(left) => left.into_reply(reply),
                Either::Right(right) => right.into_reply(reply),
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

pub(crate) fn into_work<A, M, R>(result: R, reply: DispatchReply<M::Reply>) -> ReplyWork<A>
where
    A: Actor,
    M: Message,
    R: IntoReply<A, M>,
{
    sealed::IntoReply::into_reply(result, reply)
}
