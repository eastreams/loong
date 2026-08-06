//! Reply scheduling strategies.
//!
//! A [`Handler`](crate::Handler) chooses one strategy before returning.
//! [`ReplyExt::ready`] completes during dispatch.
//! A bare [`Future`] starts an owned Tokio task.
//! [`InterleavedFutureExt::interleaved`] requires [`HasInterleaving`].
//! Actors without that capability allocate no interleaved reply queue.
//! Fixed and dynamic configurations bound active interleaved replies.
//! Unbounded configurations may retain arbitrarily many active replies.
//! [`ReplyExt::exclusive`] pauses other actor-aware work.
//! Exclusive scheduling needs no interleaving capability.
//! [`SyncHandler`](crate::SyncHandler) selects ready scheduling automatically.
//!
//! A bare `Future<Output = M::Reply> + Send + 'static` selects owned scheduling.
//! It cannot retain actor or scope borrows from its handler.
//! Move owned handles into it.
//! Recreate borrowed views inside the future.
//! Tokio polls it in a separate task.
//! Dispatched owned tasks are unbounded.
//! Mailbox capacity does not bound them.
//! The actor tracks each task until it stops.
//! Stop and Drain wait for every task.
//! Kill and actor failure request cancellation, then wait.
//! Cancellation takes effect between user polls.
//! It cannot interrupt synchronous code or user destructors.
//!
//! The actor task polls actor-aware replies.
//! Mailbox, interleaved, and child-exit work progress fairly.
//! Tokio schedules owned future polls independently.
//! Neither execution path promises poll or completion order.
//!
//! Lifecycle changes are checked before scheduled actor work.
//! Owned tasks check the same authoritative state.
//! Exclusive work pauses mailbox and actor-aware peers.
//! Already-dispatched owned replies remain Tokio-scheduled.
//!
//! A panic in handler dispatch or any reply poll fails the actor.
//! Owned future destruction follows the same rule.
//! Kill wins if it commits first.
//! [`Shutdown::Kill`](crate::Shutdown::Kill) drops active replies after running polls return.
//! Reply completion and Kill share one lifecycle decision.
//! Completion first delivers `Ok`.
//! Kill first reports
//! [`CallError::DuringDispatch`](crate::CallError::DuringDispatch).

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{
    Actor, ActorFuture, ActorScope, HasInterleaving, HasMailbox, Message,
    mailbox::DispatchReply,
    owned::OwnedTasks,
    scheduling::{ActorScheduler, InterleavedScheduler, ReplyScheduler, Seal},
};

/// Extension methods that select explicit reply scheduling strategies.
///
/// These methods wrap values without boxing or spawning.
/// The returned wrapper selects a scheduling strategy.
/// [`IntoReply`] checks its actor, message, and reply types.
pub trait ReplyExt: Sized {
    /// Creates a reply from a value produced during synchronous handler dispatch.
    ///
    /// The value is submitted after its handler returns.
    /// No future remains after this dispatch.
    /// Dispatch may wait for configured interleaved capacity.
    /// The runtime learns the strategy only after calling the handler.
    ///
    /// Prefer [`SyncHandler`](crate::SyncHandler) when every invocation returns
    /// an immediate value. Use this method inside [`Handler`](crate::Handler)
    /// when runtime branching requires explicit ready scheduling.
    ///
    /// Prefer `value.ready()` for an already-produced value.
    /// [`std::future::ready(value)`](std::future::ready) creates an ordinary [`Future`].
    /// Returning it selects owned scheduling.
    /// That spawns a task and adds a cancellation point.
    /// Use `std::future::ready` only when composition needs a future.
    /// For example, call
    /// [`IntoActorFuture::into_actor`](crate::IntoActorFuture::into_actor).
    ///
    /// Kill can commit after dispatch begins but before this value is submitted,
    /// including from the handler itself. In that case the caller receives
    /// [`CallError::DuringDispatch`](crate::CallError::DuringDispatch), and the
    /// value is dropped.
    fn ready(self) -> Ready<Self> {
        Ready { value: self }
    }

    /// Creates an actor-aware reply that reserves actor-aware execution until done.
    ///
    /// Its [`ActorFuture`] receives fresh actor and scope borrows each poll.
    /// It cannot retain those borrows across `Pending`.
    /// While this reply exists, the runtime pauses mailbox dispatch.
    /// It also pauses interleaved replies and child-exit hooks.
    /// Already-dispatched owned futures continue making progress.
    /// Kill may drop this reply after its current poll.
    /// This strategy does not require [`HasInterleaving`].
    fn exclusive(self) -> Exclusive<Self> {
        Exclusive { future: self }
    }
}

impl<T> ReplyExt for T {}

/// Selects interleaved scheduling for a capable actor.
///
/// The [`ActorFuture`] receives temporary actor borrows per poll.
/// Those borrows end whenever the poll returns.
/// `Pending` allows eligible mailbox and child-exit work.
/// It also allows other interleaved replies.
/// An active [`exclusive`](ReplyExt::exclusive) reply pauses these polls.
/// Each active reply consumes one configured slot.
/// Unbounded interleaving may retain arbitrarily many replies.
pub trait InterleavedFutureExt<A>: ActorFuture<A> + Sized
where
    A: HasInterleaving,
{
    /// Creates an actor-aware reply that yields between polls.
    fn interleaved(self) -> Interleaved<A, Self> {
        Interleaved {
            actor: PhantomData,
            future: self,
        }
    }
}

impl<A, F> InterleavedFutureExt<A> for F
where
    A: HasInterleaving,
    F: ActorFuture<A>,
{
}

/// An immediately completed reply created by [`ReplyExt::ready`].
///
/// See [`ReplyExt::ready`] for its dispatch, capacity, and Kill behavior.
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Ready<R> {
    value: R,
}

/// An interleaved reply created by [`InterleavedFutureExt::interleaved`].
///
/// See [`InterleavedFutureExt::interleaved`] for scheduling behavior.
#[derive(Debug)]
#[must_use = "a reply must be returned from a handler"]
pub struct Interleaved<A: HasInterleaving, F> {
    actor: PhantomData<fn() -> A>,
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
/// private to the runtime. Use this trait as an opaque handler return bound.
/// Use the extension traits for explicit strategies.
/// Return a bare [`Future`] for owned scheduling.
///
/// Downstream crates cannot add reply strategies:
///
/// ```compile_fail,E0277
/// use loong_actor::{Actor, ActorScope, IntoReply, Message, actor};
///
/// struct MyActor;
/// #[actor]
/// impl Actor for MyActor {
///     type SpawnArgs = Self;
///
///     async fn init(actor: Self, _scope: &mut ActorScope<'_, Self>) -> Self {
///         actor
///     }
/// }
///
/// #[derive(Message)]
/// struct MyMessage;
///
/// struct ForeignReply;
/// impl IntoReply<MyActor, MyMessage> for ForeignReply {}
/// ```
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
        fn handle(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        );
    }

    impl<A, M> HandleReply<A, M> for Ready<M::Reply>
    where
        A: Actor,
        M: Message,
    {
        fn handle(
            self,
            _owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            reply.complete(self.value);
        }
    }

    impl<A, M, F> HandleReply<A, M> for F
    where
        A: Actor,
        M: Message,
        F: Future<Output = M::Reply> + Send + 'static,
    {
        fn handle(
            self,
            owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            owned.spawn(CompleteReply::new(self, reply.into_owned()));
        }
    }

    impl<A, M, F> HandleReply<A, M> for Interleaved<A, F>
    where
        A: HasInterleaving,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn handle(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            scheduler.__push_interleaved(Seal, CompleteReply::new(self.future, reply.into_owned()));
        }
    }

    impl<A, M, F> HandleReply<A, M> for Exclusive<F>
    where
        A: HasMailbox,
        M: Message,
        F: ActorFuture<A, Output = M::Reply> + Send + 'static,
    {
        fn handle(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            scheduler.__push_exclusive(Seal, CompleteReply::new(self.future, reply.into_owned()));
        }
    }

    impl<A, M, L, R> HandleReply<A, M> for Either<L, R>
    where
        A: Actor,
        M: Message,
        L: HandleReply<A, M>,
        R: HandleReply<A, M>,
    {
        fn handle(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            match self {
                Either::Left(left) => left.handle(owned, scheduler, reply),
                Either::Right(right) => right.handle(owned, scheduler, reply),
            }
        }
    }
}

pin_project! {
    struct CompleteReply<A: Actor, F, R> {
        // Report cancellation before running the user future's Drop.
        reply: Option<DispatchReply<'static, A, R>>,
        #[pin]
        future: F,
    }
}

impl<A: Actor, F, R> CompleteReply<A, F, R> {
    fn new(future: F, reply: DispatchReply<'static, A, R>) -> Self {
        Self {
            reply: Some(reply),
            future,
        }
    }
}

impl<A, F, R> Future for CompleteReply<A, F, R>
where
    A: Actor,
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

impl<A, F, R> ActorFuture<A> for CompleteReply<A, F, R>
where
    A: Actor,
    F: ActorFuture<A, Output = R>,
    R: Send + 'static,
{
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
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
