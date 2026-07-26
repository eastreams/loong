use std::{fmt, future::Future, pin::Pin, sync::Weak, task};

use tokio::sync::{mpsc, watch};

use crate::{
    Actor, CallError, ExitReason, Handler, Message, TryCallError, TryCallErrorKind,
    mailbox::{ActorMailbox, CallEnvelope, DynEnvelope, Mode, ReplyReceiver},
};

/// A cloneable address that can communicate with, but does not own, an actor.
///
/// Keeping any number of addresses alive does not delay owner-initiated
/// shutdown. Admission waits are woken as soon as shutdown closes the mailbox.
pub struct ActorRef<A: Actor> {
    mailbox: Weak<ActorMailbox<A>>,
    exit: watch::Receiver<Option<ExitReason>>,
}

impl<A: Actor> ActorRef<A> {
    pub(crate) fn new(
        mailbox: Weak<ActorMailbox<A>>,
        exit: watch::Receiver<Option<ExitReason>>,
    ) -> Self {
        Self { mailbox, exit }
    }

    /// Sends a typed request, waiting for bounded mailbox capacity if needed.
    ///
    /// Admission and shutdown have a single linearization point. [`Closed`](
    /// CallError::Closed) therefore guarantees that the handler was never
    /// invoked. Dropping this future before dispatch lets the runtime skip the
    /// request; dropping it after dispatch does not cancel handler effects.
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
        let sender = mailbox.sender.clone();
        let mut mode = mailbox.control.subscribe_mode();

        if !mailbox.control.is_running() {
            return Err(CallError::Closed);
        }

        let reserve = sender.reserve_owned();
        tokio::pin!(reserve);

        let permit = loop {
            tokio::select! {
                biased;
                changed = mode.changed() => {
                    if changed.is_err() || !mailbox.control.is_running() {
                        return Err(CallError::Closed);
                    }
                }
                reserved = &mut reserve => {
                    break reserved.map_err(|_| CallError::Closed)?;
                }
            }
        };

        let mut message = Some(message);
        let response = mailbox
            .control
            .admit(|| {
                let message = message
                    .take()
                    .expect("admission commits a message at most once");
                let (envelope, response) = CallEnvelope::new(message, mailbox.control.clone());
                drop(permit.send(Box::new(envelope) as DynEnvelope<A>));
                response
            })
            .map_err(|()| CallError::Closed)?;

        Response::new(response).await
    }

    /// Attempts immediate bounded admission without waiting for capacity.
    ///
    /// On failure the returned error retains the original, uncommitted message.
    pub fn try_call<M>(&self, message: M) -> Result<Response<M::Reply>, TryCallError<M>>
    where
        A: Handler<M>,
        M: Message,
    {
        let Some(mailbox) = self.mailbox.upgrade() else {
            return Err(TryCallError::new(TryCallErrorKind::Closed, message));
        };

        let sender = mailbox.sender.clone();
        let mut message = Some(message);
        let admitted = mailbox.control.admit(|| match sender.try_reserve_owned() {
            Ok(permit) => {
                let message = message
                    .take()
                    .expect("admission commits a message at most once");
                let (envelope, response) = CallEnvelope::new(message, mailbox.control.clone());
                drop(permit.send(Box::new(envelope) as DynEnvelope<A>));
                Ok(response)
            }
            Err(mpsc::error::TrySendError::Full(_)) => Err(TryCallErrorKind::Full),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(TryCallErrorKind::Closed),
        });

        match admitted {
            Ok(Ok(response)) => Ok(Response::new(response)),
            Ok(Err(kind)) => Err(TryCallError::new(
                kind,
                message.expect("failed admission retains the message"),
            )),
            Err(()) => Err(TryCallError::new(
                TryCallErrorKind::Closed,
                message.expect("closed admission retains the message"),
            )),
        }
    }

    /// Returns the exit reason if the actor has already terminated.
    pub fn exit_reason(&self) -> Option<ExitReason> {
        *self.exit.borrow()
    }

    /// Waits until the actor publishes its terminal event.
    ///
    /// Stop, Drain, Kill, and contained panic publish only after the owned
    /// subtree terminates. [`ExitReason::Aborted`] is weaker: executor teardown
    /// cannot await from `Drop`, so descendants have received Kill but may still
    /// be terminating.
    pub async fn closed(&self) -> ExitReason {
        let mut exit = self.exit.clone();
        loop {
            if let Some(reason) = *exit.borrow_and_update() {
                return reason;
            }

            exit.changed()
                .await
                .expect("the actor task publishes an exit reason before closing");
        }
    }
}

impl<A: Actor> Clone for ActorRef<A> {
    fn clone(&self) -> Self {
        Self {
            mailbox: self.mailbox.clone(),
            exit: self.exit.clone(),
        }
    }
}

impl<A: Actor> fmt::Debug for ActorRef<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorRef")
            .field("exit_reason", &self.exit_reason())
            .finish_non_exhaustive()
    }
}

/// The typed reply of an accepted [`ActorRef::try_call`] request.
///
/// Dropping a queued response permits the runtime to skip its handler. Once the
/// handler starts, dropping the response only abandons the result.
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

pub(crate) async fn wait_for_kill(mode: &mut watch::Receiver<Mode>) {
    loop {
        if matches!(
            *mode.borrow_and_update(),
            Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited
        ) {
            return;
        }

        if mode.changed().await.is_err() {
            return;
        }
    }
}
