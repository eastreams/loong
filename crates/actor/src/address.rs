use std::{fmt, future::Future, pin::Pin, sync::Arc, task};

use crate::{
    Actor, CallError, DispatchHandler, ExitStatus, HasReply, Message, SendError, SendToError,
    Shutdown, ShutdownStatus, StreamHandler, StreamMessage, TryCallError, TryCallErrorKind,
    TrySendError, TrySendErrorKind, Writer,
    actor::{HasInterleaving, HasMailbox},
    mailbox::{
        ActorInner, CallEnvelope, Mode, ReplyReceiver, SendEnvelope, StreamToEnvelope,
        poll_with_panic_safe_waker,
    },
    transport::{MessageConfig, MessageReservation, MessageSender, TryReserveError},
};

/// A cloneable actor handle.
///
/// Every handle can request shutdown and observe terminal state.
/// A [`HasMailbox`] handle can also send typed messages.
/// A handle does not own lifecycle.
/// Keeping one alive does not delay owner-initiated shutdown.
/// Use [`ActorRef::recipient`] when a caller needs one message capability
/// without exposing the actor's concrete type.
pub struct ActorRef<A: Actor>(pub(crate) Arc<ActorInner<A>>);

mod private {
    pub trait Sealed {}
}

/// A cloneable, non-owning handle capability for one message type.
///
/// [`ActorRef`] implements this directly, and [`ActorRef::recipient`] erases
/// the actor type into `Arc<dyn Recipient<M>>`. This capability has no
/// lifecycle or shutdown authority; dropping it does not affect the actor.
///
/// Boxed futures keep the trait dyn-compatible for `Arc<dyn Recipient<M>>`.
///
/// The private `Sealed` supertrait keeps this a runtime capability that only
/// this crate can implement, so the blanket `Writer` impl does not overlap the
/// `mpsc::Sender` writer.
pub trait Recipient<M: Message>: Send + Sync + private::Sealed {
    /// Sends a typed request, waiting for mailbox capacity if needed.
    ///
    /// It follows [`ActorRef::call`] admission, cancellation, and shutdown
    /// rules.
    fn call<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<M::Reply, CallError>> + Send + 'a>>
    where
        M: HasReply;

    /// Attempts immediate admission without waiting for mailbox capacity.
    ///
    /// It follows [`ActorRef::try_call`] admission and error rules.
    fn try_call(&self, message: M) -> Result<Response<M::Reply>, TryCallError<M>>
    where
        M: HasReply;

    /// Sends a one-way message, waiting for mailbox capacity if needed.
    ///
    /// It follows [`ActorRef::send`] admission and shutdown rules.
    fn send<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<(), SendError<M>>> + Send + 'a>>
    where
        M: Message<Reply = ()>;

    /// Attempts immediate admission of a one-way message.
    ///
    /// It follows [`ActorRef::try_send`] admission and error rules.
    fn try_send(&self, message: M) -> Result<(), TrySendError<M>>
    where
        M: Message<Reply = ()>;
}

impl<A: Actor> private::Sealed for ActorRef<A> {}

impl<M: Message> private::Sealed for Arc<dyn Recipient<M>> {}

impl<A: Actor> ActorRef<A> {
    pub(crate) fn new(inner: Arc<ActorInner<A>>) -> Self {
        Self(inner)
    }

    /// Erases the actor type while retaining the capability for `M`.
    ///
    /// The returned handle is cloneable and non-owning. It can call `M` and,
    /// for unit-reply `M`, also send it. It has no lifecycle methods.
    #[must_use]
    pub fn recipient<M>(&self) -> Arc<dyn Recipient<M>>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message,
    {
        let recipient: Arc<dyn Recipient<M>> = Arc::new(self.clone());
        recipient
    }

    /// Sends a typed request, waiting for bounded mailbox capacity if needed.
    ///
    /// Admission and shutdown have a single commit point. [`CallError::Closed`]
    /// therefore means the message was not accepted and its handler was never
    /// invoked. Unlike [`try_call`](Self::try_call), this method does not return
    /// the message when admission fails.
    ///
    /// After admission, the request waits for actor initialization.
    /// It then waits in mailbox order for dispatch capacity.
    /// Dropping this future while queued may skip its handler.
    /// After dispatch, dropping it abandons only the result.
    /// Handler effects and selected reply work continue.
    ///
    /// After dispatch, successful completion and lifecycle interruption also
    /// have one commit point. Completion first returns `Ok`, even if Kill follows
    /// immediately. Kill, panic, or executor teardown first returns
    /// [`CallError::DuringDispatch`], including when the handler selected
    /// [`ReplyExt::ready`](crate::ReplyExt::ready).
    ///
    /// This method has no built-in deadline and can wait indefinitely while a
    /// running actor or its mailbox makes no progress. An external timeout drops
    /// the call; if dispatch already began, the handler still continues.
    pub async fn call<M>(&self, message: M) -> Result<M::Reply, CallError>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message + HasReply,
    {
        let response = match self.try_call(message) {
            Ok(response) => response,
            Err(error) if error.kind() == TryCallErrorKind::Closed => {
                return Err(CallError::Closed);
            }
            Err(error) => {
                let message = error.into_message();
                let reservation = match reserve_owned_capacity(&self.0).await {
                    CapacityReservation::Reserved(reservation) => reservation,
                    CapacityReservation::Closed => return Err(CallError::Closed),
                    CapacityReservation::TransportClosed => {
                        self.fail_transport_closed(message);
                    }
                };
                match self.admit_call(reservation, message) {
                    Ok(response) => response,
                    Err(_message) => return Err(CallError::Closed),
                }
            }
        };

        response.await
    }

    /// Sends a stream message with a caller-provided item writer and waits for
    /// its final value.
    ///
    /// Unlike [`call`](Self::call), this method does not create the runtime item
    /// channel or return a [`StreamReply`](crate::reply::StreamReply). `out` is
    /// any [`Writer`] such as an [`ActorRef`] or [`Recipient`](crate::Recipient),
    /// and the handler writes items straight to it. The final value is delivered
    /// as the ordinary call reply.
    ///
    /// This method has no built-in deadline and can wait indefinitely while a
    /// running actor or its mailbox makes no progress. Dropping the returned
    /// future abandons the call; if dispatch already began, the handler still
    /// continues.
    pub async fn call_to<M, W>(&self, message: M, out: W) -> Result<M::Final, CallError>
    where
        A: StreamHandler<M> + HasInterleaving,
        M: StreamMessage,
        W: Writer<M::Item> + Send + 'static,
    {
        let reservation = match reserve_owned_capacity(&self.0).await {
            CapacityReservation::Reserved(reservation) => reservation,
            CapacityReservation::Closed => return Err(CallError::Closed),
            CapacityReservation::TransportClosed => {
                self.fail_transport_closed((message, out));
            }
        };

        match self.admit_stream_call(reservation, message, out) {
            Ok(response) => response.await,
            Err(_) => Err(CallError::Closed),
        }
    }

    /// Sends a one-way message, waiting for bounded mailbox capacity if needed.
    ///
    /// `Ok(())` means admission committed.
    /// It may return while actor initialization remains pending.
    /// It does not wait for handler or reply work.
    /// Once accepted, the runtime owns the message.
    /// It then follows [`call`](Self::call) dispatch rules.
    /// Stop or Kill may still discard queued work.
    ///
    /// Unlike dropping a queued [`Response`], returning from this method cannot
    /// abandon the message: one-way envelopes have no response receiver. If
    /// shutdown wins before admission, the error retains the uncommitted message
    /// and its handler is never invoked.
    ///
    /// This method has no built-in deadline and can wait indefinitely while a
    /// running actor or its mailbox makes no progress. Cancelling the future
    /// before it returns drops the still-uncommitted message; use
    /// [`try_send`](Self::try_send) when capacity failure must return the message.
    pub async fn send<M>(&self, message: M) -> Result<(), SendError<M>>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message<Reply = ()>,
    {
        match self.try_send(message) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == TrySendErrorKind::Closed => {
                Err(SendError::new(error.into_message()))
            }
            Err(error) => {
                let message = error.into_message();
                let reservation = match reserve_owned_capacity(&self.0).await {
                    CapacityReservation::Reserved(reservation) => reservation,
                    CapacityReservation::Closed => return Err(SendError::new(message)),
                    CapacityReservation::TransportClosed => {
                        self.fail_transport_closed(message);
                    }
                };
                self.admit_send(reservation, message)
                    .map_err(SendError::new)
            }
        }
    }

    /// Sends a stream message with a caller-provided item writer without
    /// waiting for its final value.
    ///
    /// Like [`send`](Self::send), `Ok(())` means admission committed, not that
    /// the handler has run or finished. The handler writes items straight to
    /// `out`; the final value is produced and dropped.
    pub async fn send_to<M, W>(&self, message: M, out: W) -> Result<(), SendToError<M, W>>
    where
        A: StreamHandler<M> + HasInterleaving,
        M: StreamMessage,
        W: Writer<M::Item> + Send + 'static,
    {
        let reservation = match reserve_owned_capacity(&self.0).await {
            CapacityReservation::Reserved(reservation) => reservation,
            CapacityReservation::Closed => return Err(SendToError::new(message, out)),
            CapacityReservation::TransportClosed => {
                self.fail_transport_closed((message, out));
            }
        };

        self.admit_stream_send(reservation, message, out)
            .map_err(|(message, out)| SendToError::new(message, out))
    }

    /// Attempts immediate admission without waiting for capacity.
    ///
    /// Success means the message was accepted, not that its handler has run. The
    /// returned [`Response`] follows the same queued, dispatch, completion, and
    /// cancellation behavior as [`call`](Self::call).
    ///
    /// On failure the returned error retains the original, uncommitted message.
    /// [`TryCallErrorKind::Full`] means no mailbox slot was immediately available;
    /// [`TryCallErrorKind::Closed`] means lifecycle shutdown had closed admission.
    pub fn try_call<M>(&self, message: M) -> Result<Response<M::Reply>, TryCallError<M>>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message + HasReply,
    {
        let inner = &self.0;

        let permit = match inner.sender.try_reserve() {
            Ok(permit) => permit,
            Err(TryReserveError::Full) => {
                let kind = if inner.control.is_running() {
                    TryCallErrorKind::Full
                } else {
                    TryCallErrorKind::Closed
                };
                return Err(TryCallError::new(kind, message));
            }
            Err(TryReserveError::Closed) => {
                if inner.control.is_running() {
                    self.fail_transport_closed(message);
                }
                return Err(TryCallError::new(TryCallErrorKind::Closed, message));
            }
        };

        self.admit_call(permit, message)
            .map_err(|message| TryCallError::new(TryCallErrorKind::Closed, message))
    }

    /// Attempts immediate admission of a one-way message.
    ///
    /// Success means the message was accepted, not that its handler has run.
    /// Accepted work has no response receiver and therefore cannot be abandoned
    /// by the sender. It otherwise follows the same dispatch, reply scheduling,
    /// and lifecycle rules as [`send`](Self::send).
    ///
    /// On failure the returned error retains the original, uncommitted message.
    /// [`TrySendErrorKind::Full`] means no mailbox slot was immediately available;
    /// [`TrySendErrorKind::Closed`] means lifecycle shutdown had closed admission.
    pub fn try_send<M>(&self, message: M) -> Result<(), TrySendError<M>>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message<Reply = ()>,
    {
        let inner = &self.0;

        let permit = match inner.sender.try_reserve() {
            Ok(permit) => permit,
            Err(TryReserveError::Full) => {
                let kind = if inner.control.is_running() {
                    TrySendErrorKind::Full
                } else {
                    TrySendErrorKind::Closed
                };
                return Err(TrySendError::new(kind, message));
            }
            Err(TryReserveError::Closed) => {
                if inner.control.is_running() {
                    self.fail_transport_closed(message);
                }
                return Err(TrySendError::new(TrySendErrorKind::Closed, message));
            }
        };

        self.admit_send(permit, message)
            .map_err(|message| TrySendError::new(TrySendErrorKind::Closed, message))
    }

    /// Requests Stop, Drain, or Kill without taking lifecycle ownership.
    ///
    /// The unique owner still requests Kill when dropped.
    /// Any handle may submit an earlier lifecycle decision.
    #[must_use]
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.0.control.request(shutdown)
    }

    /// Returns a non-waiting snapshot of the actor's terminal status.
    ///
    /// `None` includes both a running actor and an actor still completing
    /// shutdown. Use [`closed`](Self::closed) to wait for terminal publication.
    #[must_use]
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.0.control.exit_status()
    }

    /// Waits until the actor publishes its terminal event.
    ///
    /// This method observes lifecycle state and does not initiate shutdown.
    /// Dropping the returned future does not affect the actor.
    ///
    /// The status's reason describes only this actor.
    /// Its subtree status reports the runtime's termination guarantee.
    pub async fn closed(&self) -> ExitStatus {
        let mut mode = self.0.control.subscribe_mode();
        loop {
            if let Mode::Exited(status) = *mode.borrow_and_update() {
                return status;
            }

            poll_with_panic_safe_waker(mode.changed())
                .await
                .expect("the actor task publishes an exit status before closing");
        }
    }

    /// Builds and admits a call with either reservation ownership shape.
    fn admit_call<M, R>(&self, reservation: R, message: M) -> Result<Response<M::Reply>, M>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message,
        R: MessageReservation<A>,
    {
        let (envelope, response) = CallEnvelope::new(message);
        match self.0.admit(reservation, Box::new(envelope)) {
            Ok(()) => Ok(Response::new(response)),
            Err((reservation, envelope)) => {
                drop(reservation);
                drop(response);
                Err((*envelope).into_message())
            }
        }
    }

    /// Builds and admits a one-way envelope with either reservation shape.
    fn admit_send<M, R>(&self, reservation: R, message: M) -> Result<(), M>
    where
        A: DispatchHandler<M, M::Kind>,
        M: Message<Reply = ()>,
        R: MessageReservation<A>,
    {
        let envelope = Box::new(SendEnvelope::new(message));
        match self.0.admit(reservation, envelope) {
            Ok(()) => Ok(()),
            Err((reservation, envelope)) => {
                drop(reservation);
                Err((*envelope).into_message())
            }
        }
    }

    /// Builds and admits a stream-call envelope carrying a caller writer.
    fn admit_stream_call<M, W, R>(
        &self,
        reservation: R,
        message: M,
        out: W,
    ) -> Result<Response<M::Final>, (M, W)>
    where
        A: StreamHandler<M> + HasInterleaving,
        M: StreamMessage,
        W: Writer<M::Item> + Send + 'static,
        R: MessageReservation<A>,
    {
        let (envelope, response) = StreamToEnvelope::new_call(message, out);
        match self.0.admit(reservation, Box::new(envelope)) {
            Ok(()) => Ok(Response::new(response)),
            Err((reservation, envelope)) => {
                drop(reservation);
                drop(response);
                Err((*envelope).into_parts())
            }
        }
    }

    /// Builds and admits a one-way stream envelope carrying a caller writer.
    fn admit_stream_send<M, W, R>(&self, reservation: R, message: M, out: W) -> Result<(), (M, W)>
    where
        A: StreamHandler<M> + HasInterleaving,
        M: StreamMessage,
        W: Writer<M::Item> + Send + 'static,
        R: MessageReservation<A>,
    {
        let envelope = Box::new(StreamToEnvelope::new_send(message, out));
        match self.0.admit(reservation, envelope) {
            Ok(()) => Ok(()),
            Err((reservation, envelope)) => {
                drop(reservation);
                Err((*envelope).into_parts())
            }
        }
    }

    /// Contains an unaccepted value before exposing transport failure.
    fn fail_transport_closed<T>(&self, value: T) -> ! {
        // The reservation failed, so erasure and admission never began.
        self.0.control.begin_failure();
        self.0.control.drop_user_value(value);
        panic!("message transport closed while actor was running");
    }
}

enum CapacityReservation<R> {
    Reserved(R),
    Closed,
    TransportClosed,
}

/// Waits for transport capacity without retaining a borrowed reservation.
#[expect(
    clippy::manual_async_fn,
    reason = "the explicit Send bound closes a generic HRTB gap"
)]
fn reserve_owned_capacity<A>(
    inner: &ActorInner<A>,
) -> impl Future<
    Output = CapacityReservation<
        <<A as MessageConfig>::Sender as MessageSender<A>>::OwnedReservation,
    >,
>
+ Send
+ '_
+ use<'_, A>
where
    A: HasMailbox,
{
    async move {
        let mut mode = inner.control.subscribe_mode();
        if !inner.control.is_running() {
            return CapacityReservation::Closed;
        }

        let reservation = poll_with_panic_safe_waker(async {
            tokio::select! {
                biased;
                reservation = inner.sender.reserve_owned() => {
                    match reservation {
                        Some(reservation) => CapacityReservation::Reserved(reservation),
                        None => CapacityReservation::TransportClosed,
                    }
                },
                _ = mode.changed() => CapacityReservation::Closed,
            }
        })
        .await;

        match reservation {
            CapacityReservation::TransportClosed if !inner.control.is_running() => {
                CapacityReservation::Closed
            }
            reservation => reservation,
        }
    }
}

impl<A: Actor> Clone for ActorRef<A> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<A: Actor> fmt::Debug for ActorRef<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorRef")
            .field("exit_status", &self.exit_status())
            .finish_non_exhaustive()
    }
}

impl<A, M> Recipient<M> for ActorRef<A>
where
    A: Actor + DispatchHandler<M, M::Kind>,
    M: Message,
{
    fn call<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<M::Reply, CallError>> + Send + 'a>>
    where
        M: HasReply,
    {
        Box::pin(ActorRef::call(self, message))
    }

    fn try_call(&self, message: M) -> Result<Response<M::Reply>, TryCallError<M>>
    where
        M: HasReply,
    {
        ActorRef::try_call(self, message)
    }

    fn send<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<(), SendError<M>>> + Send + 'a>>
    where
        M: Message<Reply = ()>,
    {
        Box::pin(ActorRef::send(self, message))
    }

    fn try_send(&self, message: M) -> Result<(), TrySendError<M>>
    where
        M: Message<Reply = ()>,
    {
        ActorRef::try_send(self, message)
    }
}

impl<M: Message> Recipient<M> for Arc<dyn Recipient<M>> {
    fn call<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<M::Reply, CallError>> + Send + 'a>>
    where
        M: HasReply,
    {
        self.as_ref().call(message)
    }

    fn try_call(&self, message: M) -> Result<Response<M::Reply>, TryCallError<M>>
    where
        M: HasReply,
    {
        self.as_ref().try_call(message)
    }

    fn send<'a>(
        &'a self,
        message: M,
    ) -> Pin<Box<dyn Future<Output = Result<(), SendError<M>>> + Send + 'a>>
    where
        M: Message<Reply = ()>,
    {
        self.as_ref().send(message)
    }

    fn try_send(&self, message: M) -> Result<(), TrySendError<M>>
    where
        M: Message<Reply = ()>,
    {
        self.as_ref().try_send(message)
    }
}

// A unit-reply message handle is also a `Writer`: `write` recovers the item
// from `send`'s `SendError` when admission is closed.
impl<M, R> Writer<M> for R
where
    M: Message<Reply = ()>,
    R: Recipient<M>,
{
    async fn write(&mut self, item: M) -> Result<(), M> {
        Recipient::send(&*self, item)
            .await
            .map_err(SendError::into_message)
    }
}

/// The typed reply of an accepted [`ActorRef::try_call`] or
/// [`Recipient::try_call`] request.
///
/// The message has committed to the mailbox, but its handler may not have run.
/// It may remain queued while initialization is pending.
/// Dropping a queued response permits the runtime to skip that handler. Once
/// dispatch starts, dropping the response only abandons the result; handler and
/// reply effects continue.
///
/// Awaiting this value returns the phase-aware [`CallError`] contract. In
/// particular, completion committed before Kill returns `Ok`, while Kill
/// committed first returns [`CallError::DuringDispatch`].
#[must_use = "dropping a queued response may abandon its message"]
pub struct Response<R> {
    receiver: ReplyReceiver<R>,
}

impl<R> Response<R> {
    fn new(receiver: ReplyReceiver<R>) -> Self {
        Self { receiver }
    }
}

impl<R> Future for Response<R> {
    type Output = Result<R, CallError>;

    fn poll(self: Pin<&mut Self>, context: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(context) {
            task::Poll::Ready(Ok(response)) => task::Poll::Ready(response),
            task::Poll::Ready(Err(_)) => task::Poll::Ready(Err(CallError::ResponseLost)),
            task::Poll::Pending => task::Poll::Pending,
        }
    }
}

impl<R> fmt::Debug for Response<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Response(..)")
    }
}
