use std::{fmt, future::Future, pin::Pin, sync::Weak, task};

use tokio::sync::{mpsc, watch};

use crate::{
    Actor, CallError, ExitStatus, Handler, Message, SendError, TryCallError, TryCallErrorKind,
    TrySendError, TrySendErrorKind,
    mailbox::{ActorMailbox, CallEnvelope, Mode, ReplyReceiver, SendEnvelope, mode_changed},
};

/// A cloneable address that can communicate with, but does not own, an actor.
///
/// Keeping any number of addresses alive does not delay owner-initiated
/// shutdown. Admission waits are woken as soon as shutdown closes admission.
/// Requests accepted by cloned addresses share the same bounded mailbox.
pub struct ActorRef<A: Actor> {
    mailbox: Weak<ActorMailbox<A>>,
    mode: watch::Receiver<Mode>,
}

impl<A: Actor> ActorRef<A> {
    pub(crate) fn new(mailbox: Weak<ActorMailbox<A>>, mode: watch::Receiver<Mode>) -> Self {
        Self { mailbox, mode }
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
        A: Handler<M>,
        M: Message,
    {
        let mailbox = self.mailbox.upgrade().ok_or(CallError::Closed)?;
        let mut mode = mailbox.control.subscribe_mode();

        if !mailbox.control.is_running() {
            return Err(CallError::Closed);
        }

        let reserve = mailbox.sender.reserve();
        tokio::pin!(reserve);

        let permit = loop {
            tokio::select! {
                biased;
                reserved = &mut reserve => {
                    break reserved.map_err(|_| CallError::Closed)?;
                }
                changed = mode_changed(&mut mode) => {
                    if changed.is_err() || !mailbox.control.is_running() {
                        return Err(CallError::Closed);
                    }
                }
            }
        };

        let (envelope, response) = CallEnvelope::new(message, mailbox.control.clone());
        match mailbox.admit(permit, Box::new(envelope)) {
            Ok(()) => {}
            Err((permit, envelope)) => {
                drop(permit);
                drop(response);
                drop((*envelope).into_message());
                return Err(CallError::Closed);
            }
        }

        Response::new(response).await
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
        A: Handler<M>,
        M: Message<Reply = ()>,
    {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return Err(SendError::new(message));
        };
        let mut mode = mailbox.control.subscribe_mode();

        if !mailbox.control.is_running() {
            return Err(SendError::new(message));
        }

        let reserve = mailbox.sender.reserve();
        tokio::pin!(reserve);

        let permit = loop {
            tokio::select! {
                biased;
                reserved = &mut reserve => {
                    match reserved {
                        Ok(permit) => break permit,
                        Err(_) => return Err(SendError::new(message)),
                    }
                }
                changed = mode_changed(&mut mode) => {
                    if changed.is_err() || !mailbox.control.is_running() {
                        return Err(SendError::new(message));
                    }
                }
            }
        };

        let envelope = Box::new(SendEnvelope::new(message, mailbox.control.clone()));
        match mailbox.admit(permit, envelope) {
            Ok(()) => Ok(()),
            Err((permit, envelope)) => {
                drop(permit);
                Err(SendError::new((*envelope).into_message()))
            }
        }
    }

    /// Attempts immediate bounded admission without waiting for capacity.
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
        A: Handler<M>,
        M: Message,
    {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return Err(TryCallError::new(TryCallErrorKind::Closed, message));
        };

        let permit = match mailbox.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                let kind = if mailbox.control.is_running() {
                    TryCallErrorKind::Full
                } else {
                    TryCallErrorKind::Closed
                };
                return Err(TryCallError::new(kind, message));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(TryCallError::new(TryCallErrorKind::Closed, message));
            }
        };

        let (envelope, response) = CallEnvelope::new(message, mailbox.control.clone());
        match mailbox.admit(permit, Box::new(envelope)) {
            Ok(()) => Ok(Response::new(response)),
            Err((permit, envelope)) => {
                drop(permit);
                drop(response);
                Err(TryCallError::new(
                    TryCallErrorKind::Closed,
                    (*envelope).into_message(),
                ))
            }
        }
    }

    /// Attempts immediate bounded admission of a one-way message.
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
        A: Handler<M>,
        M: Message<Reply = ()>,
    {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return Err(TrySendError::new(TrySendErrorKind::Closed, message));
        };

        let permit = match mailbox.sender.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(_)) => {
                let kind = if mailbox.control.is_running() {
                    TrySendErrorKind::Full
                } else {
                    TrySendErrorKind::Closed
                };
                return Err(TrySendError::new(kind, message));
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(TrySendError::new(TrySendErrorKind::Closed, message));
            }
        };

        let envelope = Box::new(SendEnvelope::new(message, mailbox.control.clone()));
        match mailbox.admit(permit, envelope) {
            Ok(()) => Ok(()),
            Err((permit, envelope)) => {
                drop(permit);
                Err(TrySendError::new(
                    TrySendErrorKind::Closed,
                    (*envelope).into_message(),
                ))
            }
        }
    }

    /// Returns a non-waiting snapshot of the actor's terminal status.
    ///
    /// `None` includes both a running actor and an actor still completing
    /// shutdown. Use [`closed`](Self::closed) to wait for terminal publication.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        match *self.mode.borrow() {
            Mode::Exited(status) => Some(status),
            _ => None,
        }
    }

    /// Waits until the actor publishes its terminal event.
    ///
    /// This method observes lifecycle state and does not initiate shutdown.
    /// Dropping the returned future does not affect the actor.
    ///
    /// The status's reason describes only this actor.
    /// Its subtree status reports the runtime's termination guarantee.
    pub async fn closed(&self) -> ExitStatus {
        let mut mode = self.mode.clone();
        loop {
            if let Mode::Exited(status) = *mode.borrow_and_update() {
                return status;
            }

            mode_changed(&mut mode)
                .await
                .expect("the actor task publishes an exit status before closing");
        }
    }
}

impl<A: Actor> Clone for ActorRef<A> {
    fn clone(&self) -> Self {
        Self {
            mailbox: self.mailbox.clone(),
            mode: self.mode.clone(),
        }
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

/// The typed reply of an accepted [`ActorRef::try_call`] request.
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
