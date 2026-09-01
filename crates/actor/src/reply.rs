//! Reply scheduling strategies.
//!
//! A [`RawHandler`](crate::RawHandler) chooses one strategy before returning.
//! [`ReplyExt::ready`] completes during dispatch.
//! A bare [`Future`] starts an owned Tokio task.
//! [`InterleavedFutureExt::interleaved`] requires [`HasInterleaving`].
//! Actors without that capability allocate no interleaved reply queue.
//! Fixed and dynamic configurations bound active interleaved replies.
//! Unbounded configurations may retain arbitrarily many active replies.
//! [`ReplyExt::exclusive`] pauses other actor-aware work.
//! Exclusive scheduling needs no interleaving capability.
//! [`Handler`](crate::Handler) schedules cx futures on the interleaved lane
//! automatically.
//!
//! A bare `Future<Output = M::Reply> + Send + 'static` selects owned scheduling.
//! It cannot retain actor or scope borrows from its handler.
//! Move owned handles into it.
//! Recreate borrowed views inside the future.
//! Tokio polls it in a separate task.
//! Dispatched owned tasks are unbounded; mailbox capacity does not bound them.
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
//! [`CallError::DuringDispatch`].

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tokio::sync::{mpsc, oneshot};

use crate::{
    Actor, ActorFuture, ActorScope, CallError, HasInterleaving, HasMailbox, Message,
    mailbox::DispatchReply,
    owned::OwnedTasks,
    scheduling::{ActorScheduler, InterleavedScheduler, ReplyScheduler, Seal},
};

mod private {
    pub trait Sealed {}
}

/// Selects the reply channel shape a [`Message`] uses.
///
/// [`DispatchHandler`](crate::DispatchHandler) is generic over this kind. The
/// runtime uses the message's [`Message::Kind`] to select the matching
/// dispatch implementation. Distinct kind values keep the public handler
/// blanket impls disjoint.
pub trait ReplyKind: private::Sealed + Send + 'static {}

/// The ordinary reply kind: one typed value returned to the caller.
pub struct SyncKind;

impl private::Sealed for SyncKind {}
impl ReplyKind for SyncKind {}

/// The streaming reply kind: a [`StreamReply`] handle returned to the caller.
pub struct StreamKind;

impl private::Sealed for StreamKind {}
impl ReplyKind for StreamKind {}

/// The raw ordinary reply kind: explicit reply strategy selection.
///
/// Messages with this kind are handled by [`RawHandler`](crate::RawHandler).
pub struct RawKind;

impl private::Sealed for RawKind {}
impl ReplyKind for RawKind {}

/// The raw streaming reply kind: explicit stream-final strategy selection.
///
/// Stream messages with this kind are handled by
/// [`RawStreamHandler`](crate::RawStreamHandler).
pub struct RawStreamKind;

impl private::Sealed for RawStreamKind {}
impl ReplyKind for RawStreamKind {}

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
    /// Use this method inside [`RawHandler`](crate::RawHandler) when runtime
    /// branching requires explicit ready scheduling.
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
    /// [`CallError::DuringDispatch`], and the
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

/// An interleaved plain-Future reply that may access actor state through the
/// [`Cx`](crate::Cx) handle captured by the future.
///
/// Created by [`Handler`](crate::Handler) dispatch, or manually through
/// [`ActorScope::cx_reply`](crate::ActorScope::cx_reply).
#[must_use = "a reply must be returned from a handler"]
pub struct CxReply<A, R> {
    pub(crate) future: std::pin::Pin<Box<dyn Future<Output = R> + Send + 'static>>,
    pub(crate) _actor: PhantomData<fn() -> A>,
}

/// Streaming counterpart of [`CxReply`].
///
/// Created by [`StreamHandler`](crate::StreamHandler) dispatch, or manually
/// through [`ActorScope::cx_stream`](crate::ActorScope::cx_stream).
#[must_use = "a reply must be returned from a handler"]
pub struct CxStream<A, R> {
    pub(crate) future: std::pin::Pin<Box<dyn Future<Output = R> + Send + 'static>>,
    pub(crate) _actor: PhantomData<fn() -> A>,
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
/// use loac::{Actor, ActorScope, IntoReply, Message, actor};
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

/// A stream-final scheduling strategy returned by a
/// [`RawStreamHandler`](crate::RawStreamHandler).
///
/// This is the streaming counterpart of [`IntoReply`]: a bare [`Future`] with
/// output `M::Final` selects owned scheduling, [`Ready`] completes the final
/// value immediately, [`Exclusive`] selects actor-aware mailbox-pausing
/// scheduling, [`Interleaved`] selects actor-aware interleaved scheduling, and
/// [`Either`] chooses between two strategies at runtime.
///
/// [`Ready`] is mainly useful as one branch of an [`Either`] when the handler
/// decides not to stream items. A message type that never streams should be an
/// ordinary message instead of a stream message.
///
/// The trait is sealed so the stream-final sender and scheduler access stay
/// private to the runtime.
pub trait IntoStreamReply<A: Actor, M: StreamReplyMessage>:
    sealed::HandleStream<A, M> + sealed::HandleStreamCall<A, M>
{
}

impl<A, M, T> IntoStreamReply<A, M> for T
where
    A: Actor,
    M: StreamReplyMessage,
    T: sealed::HandleStream<A, M> + sealed::HandleStreamCall<A, M>,
{
}

/// A caller handle for a streamed reply.
///
/// `call` returns this handle after the stream is set up. Read items with
/// [`recv`](Self::recv) or [`items`](Self::items), then wait for the final
/// value with [`finish`](Self::finish). Dropping the handle cancels the item
/// stream; the handler observes closed writes and may stop early.
///
/// [`recv`](Self::recv) and [`items`](Self::items) wait for the item stream to
/// close. A well-behaved handler closes it by dropping the writer when its
/// future finishes. If a handler leaks the writer into a detached task, the
/// item stream may stay open after the final value is ready; [`finish`](Self::finish)
/// is final-aware and still returns.
#[must_use = "a stream reply must be consumed or finished"]
#[derive(Debug)]
pub struct StreamReply<Item, Final> {
    item_rx: mpsc::Receiver<Item>,
    final_rx: oneshot::Receiver<Final>,
}

impl<Item, Final> StreamReply<Item, Final> {
    /// Receives the next streamed item, or `None` when the item stream is done.
    pub async fn recv(&mut self) -> Option<Item> {
        self.item_rx.recv().await
    }

    /// Borrows the item stream as a [`futures_util::Stream`].
    ///
    /// The borrow keeps this [`StreamReply`] alive, so the final value stays
    /// available through [`finish`](Self::finish) after the item stream ends.
    pub fn items(&mut self) -> Items<'_, Item> {
        Items {
            item_rx: &mut self.item_rx,
        }
    }

    /// Consumes this handle, discards remaining items, and returns the final
    /// reply value.
    ///
    /// The item stream is drained while waiting so a handler that fills the
    /// item channel can finish. If the final value arrives first, already
    /// buffered items are discarded and the final value is returned.
    pub async fn finish(mut self) -> Result<Final, CallError> {
        loop {
            tokio::select! {
                item = self.item_rx.recv() => {
                    if item.is_none() {
                        return self.final_rx.await.map_err(|_| CallError::ResponseLost);
                    }
                }
                result = &mut self.final_rx => {
                    while self.item_rx.try_recv().is_ok() {}
                    return result.map_err(|_| CallError::ResponseLost);
                }
            }
        }
    }
}

/// Borrowed item-stream view created by [`StreamReply::items`].
#[derive(Debug)]
pub struct Items<'a, Item> {
    item_rx: &'a mut mpsc::Receiver<Item>,
}

impl<Item> futures_util::Stream for Items<'_, Item> {
    type Item = Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().item_rx.poll_recv(cx)
    }
}

/// A stream-shaped reply: items plus a final value.
///
/// This is the kind-agnostic shape shared by normal stream messages and raw
/// stream messages. [`IntoStreamReply`] and the stream reply scheduling
/// machinery are generic over this shape.
pub trait StreamReplyMessage: Message<Reply = StreamReply<Self::Item, Self::Final>> {
    /// Type of each streamed item.
    type Item: Send + 'static;
    /// Type of the final value delivered after the stream ends.
    type Final: Send + 'static;
}

/// A [`Message`] whose reply is a stream of items plus a final value.
///
/// The derive implements this when `#[message(stream = Item, reply = Final)]`
/// is present. The blanket [`StreamHandler`](crate::StreamHandler) adaptation
/// targets [`DispatchHandler<M, StreamKind>`](crate::DispatchHandler).
/// Raw stream messages use `StreamMessage<RawStreamKind>` and adapt to
/// [`RawStreamHandler`](crate::RawStreamHandler).
pub trait StreamMessage<Kind: ReplyKind = StreamKind>:
    StreamReplyMessage + Message<Kind = Kind>
{
}

pub(crate) struct StreamDispatch<S, Item, Final> {
    strategy: S,
    item_rx: mpsc::Receiver<Item>,
    final_tx: oneshot::Sender<Final>,
    final_rx: oneshot::Receiver<Final>,
}

impl<S, Item, Final> StreamDispatch<S, Item, Final> {
    pub(crate) fn new(
        strategy: S,
        item_rx: mpsc::Receiver<Item>,
        final_tx: oneshot::Sender<Final>,
        final_rx: oneshot::Receiver<Final>,
    ) -> Self {
        Self {
            strategy,
            item_rx,
            final_tx,
            final_rx,
        }
    }
}

impl<A, M, S> sealed::HandleReply<A, M> for StreamDispatch<S, M::Item, M::Final>
where
    A: Actor,
    M: StreamReplyMessage,
    S: sealed::HandleStream<A, M>,
{
    fn handle(
        self,
        owned: &OwnedTasks<A>,
        scheduler: &mut ActorScheduler<A>,
        reply: DispatchReply<'_, A, M::Reply>,
    ) {
        let this = self;
        reply.complete(StreamReply {
            item_rx: this.item_rx,
            final_rx: this.final_rx,
        });
        this.strategy.handle_stream(owned, scheduler, this.final_tx);
    }
}

pin_project! {
    struct FinishStream<F, T> {
        #[pin]
        future: F,
        final_tx: Option<oneshot::Sender<T>>,
    }
}

impl<F, T> FinishStream<F, T> {
    fn new(future: F, final_tx: oneshot::Sender<T>) -> Self {
        Self {
            future,
            final_tx: Some(final_tx),
        }
    }
}

impl<F, T> Future for FinishStream<F, T>
where
    F: Future<Output = T>,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.future.poll(cx) {
            Poll::Ready(value) => {
                if let Some(tx) = this.final_tx.take() {
                    let _ = tx.send(value);
                }
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<A, F, T> ActorFuture<A> for FinishStream<F, T>
where
    A: Actor,
    F: ActorFuture<A, Output = T>,
    T: Send + 'static,
{
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.project();
        match this.future.poll(actor, scope, cx) {
            Poll::Ready(value) => {
                if let Some(tx) = this.final_tx.take() {
                    let _ = tx.send(value);
                }
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
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

    impl<A, M> HandleReply<A, M> for CxReply<A, M::Reply>
    where
        A: HasInterleaving,
        M: Message,
        M::Reply: Send + 'static,
    {
        fn handle(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Reply>,
        ) {
            use crate::IntoActorFuture;
            scheduler.__push_interleaved(
                Seal,
                CompleteReply::new(self.future.into_actor(), reply.into_owned()),
            );
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

    /// Stream-final scheduling dispatch.
    ///
    /// The caller reply is already completed with a [`StreamReply`] before
    /// this is called; only final-value production remains to be scheduled.
    pub trait HandleStream<A: Actor, M: StreamReplyMessage> {
        fn handle_stream(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        );
    }

    impl<A, M> HandleStream<A, M> for Ready<M::Final>
    where
        A: Actor,
        M: StreamReplyMessage,
    {
        fn handle_stream(
            self,
            _owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            let _ = final_tx.send(self.value);
        }
    }

    impl<A, M, F> HandleStream<A, M> for F
    where
        A: Actor,
        M: StreamReplyMessage,
        F: Future<Output = M::Final> + Send + 'static,
    {
        fn handle_stream(
            self,
            owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            owned.spawn(FinishStream::new(self, final_tx));
        }
    }

    impl<A, M, F> HandleStream<A, M> for Interleaved<A, F>
    where
        A: HasInterleaving,
        M: StreamReplyMessage,
        F: ActorFuture<A, Output = M::Final> + Send + 'static,
    {
        fn handle_stream(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            scheduler.__push_interleaved(Seal, FinishStream::new(self.future, final_tx));
        }
    }

    impl<A, M, F> HandleStream<A, M> for Exclusive<F>
    where
        A: HasMailbox,
        M: StreamReplyMessage,
        F: ActorFuture<A, Output = M::Final> + Send + 'static,
    {
        fn handle_stream(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            scheduler.__push_exclusive(Seal, FinishStream::new(self.future, final_tx));
        }
    }

    impl<A, M> HandleStream<A, M> for CxStream<A, M::Final>
    where
        A: HasInterleaving,
        M: StreamReplyMessage,
        M::Final: Send + 'static,
    {
        fn handle_stream(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            use crate::IntoActorFuture;
            scheduler
                .__push_interleaved(Seal, FinishStream::new(self.future.into_actor(), final_tx));
        }
    }

    impl<A, M, L, R> HandleStream<A, M> for Either<L, R>
    where
        A: Actor,
        M: StreamReplyMessage,
        L: HandleStream<A, M>,
        R: HandleStream<A, M>,
    {
        fn handle_stream(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            final_tx: oneshot::Sender<M::Final>,
        ) {
            match self {
                Either::Left(left) => left.handle_stream(owned, scheduler, final_tx),
                Either::Right(right) => right.handle_stream(owned, scheduler, final_tx),
            }
        }
    }

    /// Stream-final dispatch that completes a call reply directly.
    ///
    /// Used when the caller supplies the item writer, so there is no
    /// [`StreamReply`] handle to complete. The final value is delivered through
    /// the ordinary call reply path instead.
    pub trait HandleStreamCall<A: Actor, M: StreamReplyMessage> {
        fn handle_stream_call(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        );
    }

    impl<A, M> HandleStreamCall<A, M> for Ready<M::Final>
    where
        A: Actor,
        M: StreamReplyMessage,
    {
        fn handle_stream_call(
            self,
            _owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            reply.complete(self.value);
        }
    }

    impl<A, M, F> HandleStreamCall<A, M> for F
    where
        A: Actor,
        M: StreamReplyMessage,
        F: Future<Output = M::Final> + Send + 'static,
    {
        fn handle_stream_call(
            self,
            owned: &OwnedTasks<A>,
            _scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            owned.spawn(CompleteReply::new(self, reply.into_owned()));
        }
    }

    impl<A, M, F> HandleStreamCall<A, M> for Interleaved<A, F>
    where
        A: HasInterleaving,
        M: StreamReplyMessage,
        F: ActorFuture<A, Output = M::Final> + Send + 'static,
    {
        fn handle_stream_call(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            scheduler.__push_interleaved(Seal, CompleteReply::new(self.future, reply.into_owned()));
        }
    }

    impl<A, M, F> HandleStreamCall<A, M> for Exclusive<F>
    where
        A: HasMailbox,
        M: StreamReplyMessage,
        F: ActorFuture<A, Output = M::Final> + Send + 'static,
    {
        fn handle_stream_call(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            scheduler.__push_exclusive(Seal, CompleteReply::new(self.future, reply.into_owned()));
        }
    }

    impl<A, M> HandleStreamCall<A, M> for CxStream<A, M::Final>
    where
        A: HasInterleaving,
        M: StreamReplyMessage,
        M::Final: Send + 'static,
    {
        fn handle_stream_call(
            self,
            _owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            use crate::IntoActorFuture;
            scheduler.__push_interleaved(
                Seal,
                CompleteReply::new(self.future.into_actor(), reply.into_owned()),
            );
        }
    }

    impl<A, M, L, R> HandleStreamCall<A, M> for Either<L, R>
    where
        A: Actor,
        M: StreamReplyMessage,
        L: HandleStreamCall<A, M>,
        R: HandleStreamCall<A, M>,
    {
        fn handle_stream_call(
            self,
            owned: &OwnedTasks<A>,
            scheduler: &mut ActorScheduler<A>,
            reply: DispatchReply<'_, A, M::Final>,
        ) {
            match self {
                Either::Left(left) => left.handle_stream_call(owned, scheduler, reply),
                Either::Right(right) => right.handle_stream_call(owned, scheduler, reply),
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
