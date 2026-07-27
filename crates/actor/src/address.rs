use std::{fmt, future::Future, pin::Pin, sync::Weak, task};

use tokio::sync::{mpsc, watch};

use crate::{
    Actor, CallError, ExitReason, Handler, Message, TryCallError, TryCallErrorKind,
    mailbox::{ActorMailbox, CallEnvelope, DynEnvelope, Mode, ReplyReceiver},
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
    /// After admission, the request waits in mailbox order until in-flight
    /// capacity and reply scheduling permit dispatch. Dropping this future while
    /// the request is still queued permits the runtime to skip its handler. Once
    /// dispatch begins, dropping the future abandons only the result; synchronous
    /// handler effects and its selected reply continue.
    ///
    /// After dispatch, successful completion and lifecycle interruption also
    /// have one commit point. Completion first returns `Ok`, even if Kill follows
    /// immediately. Kill, panic, or executor teardown first returns
    /// [`CallError::DuringDispatch`], including when the handler selected
    /// [`reply::ready`](crate::reply::ready).
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

    /// Returns a non-waiting snapshot of the actor's terminal reason.
    ///
    /// `None` includes both a running actor and an actor still completing
    /// shutdown. Use [`closed`](Self::closed) to wait for terminal publication.
    pub fn exit_reason(&self) -> Option<ExitReason> {
        match *self.mode.borrow() {
            Mode::Exited(reason) => Some(reason),
            _ => None,
        }
    }

    /// Waits until the actor publishes its terminal event.
    ///
    /// This method observes lifecycle state and does not initiate shutdown.
    /// Dropping the returned future does not affect the actor.
    ///
    /// Stop, Drain, Kill, and contained panic publish only after the owned
    /// subtree terminates. [`ExitReason::Aborted`] is weaker: executor teardown
    /// cannot await from `Drop`, so descendants have received Kill but may still
    /// be terminating.
    pub async fn closed(&self) -> ExitReason {
        let mut mode = self.mode.clone();
        loop {
            if let Mode::Exited(reason) = *mode.borrow_and_update() {
                return reason;
            }

            mode.changed()
                .await
                .expect("the actor task publishes an exit reason before closing");
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
            .field("exit_reason", &self.exit_reason())
            .finish_non_exhaustive()
    }
}

/// The typed reply of an accepted [`ActorRef::try_call`] request.
///
/// The message has committed to the mailbox, but its handler may not have run.
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

pub(crate) async fn wait_for_kill(mode: &mut watch::Receiver<Mode>) {
    loop {
        if matches!(
            *mode.borrow_and_update(),
            Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_)
        ) {
            return;
        }

        if mode.changed().await.is_err() {
            return;
        }
    }
}
