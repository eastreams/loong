//! Reply scheduling strategies.
//!
//! A [`Handler`](crate::Handler) synchronously selects one of the strategies
//! described here. [`ReplyExt::ready`] completes in that dispatch call. A bare
//! [`Future`], [`ReplyExt::interleaved`], and [`ReplyExt::exclusive`] instead
//! register one active reply, which occupies a
//! [`max_in_flight`](crate::SpawnOptions::max_in_flight) slot until it completes
//! or is dropped.
//!
//! Returning a bare `Future<Output = M::Reply> + Send + 'static` selects owned
//! scheduling. The future cannot retain the handler call's actor or scope
//! borrows; clone or move owned handles into it and construct borrowed views
//! inside the future. The actor task polls it directly without `tokio::spawn`.
//! While it is pending, mailbox and actor-aware work may progress fairly, and it
//! continues to receive poll opportunities while an exclusive reply is active.
//!
//! Asynchronous replies are polled by the actor task; the runtime does not spawn
//! each one as an independent Tokio task. Without an exclusive reply, eligible
//! mailbox dispatch, owned replies, interleaved replies, and entry into
//! direct-child exit hooks are scheduled fairly. Fairness guarantees continued
//! opportunities to make progress, not a deterministic poll order or reply
//! completion order.
//!
//! Lifecycle observation sits outside that fairness domain and is checked
//! before a new fair turn and between user future polls. While an exclusive
//! reply exists, mailbox dispatch, interleaved replies, and child-exit hooks
//! pause; the exclusive reply and already-dispatched owned replies both continue
//! to receive progress opportunities.
//!
//! A panic in synchronous handler dispatch or in any reply poll causes actor
//! failure and drops its other work unless Kill commits first.
//! [`Shutdown::Kill`](crate::Shutdown::Kill) likewise drops active replies once
//! the current poll returns. Reply completion and Kill share a lifecycle
//! decision: completion first delivers `Ok`, while Kill first reports
//! [`CallError::DuringDispatch`](crate::CallError::DuringDispatch).

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{
    Actor, ActorFuture, ActorScope, Message, mailbox::DispatchReply, scheduler::ReplyScheduler,
};

/// Extension methods that select explicit reply scheduling strategies.
///
/// These methods only construct statically dispatched wrappers; they do not box
/// or spawn their values. [`IntoReply`] checks the selected wrapper against the
/// handler's actor, message, and reply types. Keeping the actor bound at that
/// boundary lets the Handler return context select `A` even when one future type
/// implements `ActorFuture<A>` for multiple actors.
pub trait ReplyExt: Sized {
    /// Creates a reply from a value produced during synchronous handler dispatch.
    ///
    /// The value is submitted immediately after the handler returns, within the
    /// same synchronous dispatch, so no future is stored and no in-flight slot
    /// remains occupied afterward. Dispatch itself still waits until an
    /// in-flight slot is available because the runtime cannot know the selected
    /// reply strategy before invoking the handler.
    ///
    /// For a value already produced by a handler, prefer `value.ready()` over
    /// [`std::future::ready(value)`](std::future::ready). The latter creates an
    /// ordinary [`Future`] and therefore selects owned scheduling when returned
    /// directly: it occupies an active slot until polled and adds another point
    /// at which Kill may commit before completion. Use `std::future::ready` when
    /// a future is intentionally needed for composition, such as before
    /// [`IntoActorFuture::into_actor`](crate::IntoActorFuture::into_actor).
    ///
    /// Kill can commit after dispatch begins but before this value is submitted,
    /// including from the handler itself. In that case the caller receives
    /// [`CallError::DuringDispatch`](crate::CallError::DuringDispatch), and the
    /// value is dropped.
    fn ready(self) -> Ready<Self> {
        Ready { value: self }
    }

    /// Creates an actor-aware reply that yields actor access between polls.
    ///
    /// The [`ActorFuture`] receives fresh temporary actor and scope borrows on
    /// each poll. Those borrows end whenever the poll returns, including on
    /// `Pending`, so eligible mailbox dispatch, other replies, and direct-child
    /// exit hooks may run before it is polled again. An active
    /// [`exclusive`](ReplyExt::exclusive) reply pauses these polls.
    fn interleaved(self) -> Interleaved<Self> {
        Interleaved { future: self }
    }

    /// Creates an actor-aware reply that reserves actor-aware execution until done.
    ///
    /// Like [`interleaved`](ReplyExt::interleaved), its [`ActorFuture`] receives
    /// fresh actor and scope borrows for each poll and cannot retain them across
    /// `Pending`. Unlike `interleaved`, the runtime does not dispatch mailbox
    /// messages, poll interleaved replies, or run direct-child exit hooks while
    /// this reply exists. Already-dispatched owned futures continue to make
    /// progress, and Kill can still drop the exclusive reply after its current
    /// poll returns.
    fn exclusive(self) -> Exclusive<Self> {
        Exclusive { future: self }
    }
}

impl<T> ReplyExt for T {}

/// An immediately completed reply created by [`ReplyExt::ready`].
///
/// See [`ReplyExt::ready`] for its dispatch, capacity, and Kill behavior.
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Ready<R> {
    value: R,
}

/// An interleaved actor-aware reply created by [`ReplyExt::interleaved`].
///
/// See [`ReplyExt::interleaved`] for its borrowing and scheduling behavior.
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Interleaved<F> {
    future: F,
}

/// An exclusive actor-aware reply created by [`ReplyExt::exclusive`].
///
/// See [`ReplyExt::exclusive`] for the work it pauses and the work that may
/// continue.
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
/// use [`ReplyExt`] for explicit strategies or return a bare [`Future`] for owned
/// scheduling.
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

    impl<A, M, F> HandleReply<A, M> for F
    where
        A: Actor,
        M: Message,
        F: Future<Output = M::Reply> + Send + 'static,
    {
        fn handle(self, scheduler: &mut ReplyScheduler<A>, reply: DispatchReply<M::Reply>) {
            scheduler.push_owned(CompleteOwnedReply::new(self, reply));
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
