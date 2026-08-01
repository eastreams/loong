use std::{error::Error, fmt};

use thiserror::Error;

use crate::ExitReason;

/// A request failure reported before a typed reply is delivered.
///
/// Variants identify the last user-visible request phase committed by the
/// runtime:
///
/// - [`Closed`](Self::Closed): admission did not commit.
/// - [`BeforeDispatch`](Self::BeforeDispatch): admission committed, but dispatch
///   did not.
/// - [`DuringDispatch`](Self::DuringDispatch): dispatch committed, but successful
///   completion did not.
///
/// Admission, dispatch, and successful completion are each ordered atomically
/// against shutdown and actor failure. If completion commits first, the caller
/// receives `Ok`. Otherwise the error identifies whether interruption happened
/// before or during dispatch. [`ResponseLost`](Self::ResponseLost) is the
/// fallback when no precise lifecycle phase reaches the response channel.
///
/// Lifecycle variants carry the interruption reason observed at cutoff.
/// That value is not a terminal snapshot.
/// Subtree confirmation may not exist yet.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CallError {
    /// The actor stopped accepting requests before this message was committed.
    ///
    /// The handler was never invoked. [`ActorRef::call`](crate::ActorRef::call)
    /// consumes the message on this path; use
    /// [`ActorRef::try_call`](crate::ActorRef::try_call) when the original message
    /// must be recoverable after failed admission.
    #[error("the actor is closed to new messages")]
    Closed,

    /// The message was accepted, but lifecycle shutdown or failure discarded it
    /// before its handler was invoked.
    #[error("the request was discarded before dispatch: {0}")]
    BeforeDispatch(ExitReason),

    /// The handler began, but interruption committed before successful reply
    /// completion.
    ///
    /// Synchronous handler work and earlier future polls may already have caused
    /// effects. This error is therefore not proof that retrying is safe, even for
    /// a handler that selected [`ReplyExt::ready`](crate::ReplyExt::ready).
    #[error("the request was interrupted during dispatch: {0}")]
    DuringDispatch(ExitReason),

    /// The response channel vanished without the runtime reporting a phase.
    ///
    /// The request's last committed phase is unknown, so this error does not make
    /// retrying safe.
    #[error("the actor response channel was lost")]
    ResponseLost,
}

/// A one-way message that could not commit to an actor's mailbox.
///
/// The actor had already closed admission, so the handler was never invoked.
/// The original message can be recovered with [`into_message`](Self::into_message)
/// and is safe to retry elsewhere.
#[derive(thiserror::Error)]
#[error("the actor is closed to new messages")]
pub struct SendError<M> {
    message: M,
}

impl<M> SendError<M> {
    pub(crate) const fn new(message: M) -> Self {
        Self { message }
    }

    /// Returns the message without retrying or dropping it.
    pub fn into_message(self) -> M {
        self.message
    }
}

impl<M> fmt::Debug for SendError<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SendError")
            .field("message", &"<message>")
            .finish()
    }
}

/// The reason a synchronous one-way admission attempt failed.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum TrySendErrorKind {
    /// No mailbox slot was immediately available.
    #[error("the actor mailbox is full")]
    Full,

    /// The actor no longer accepts new messages.
    #[error("the actor is closed to new messages")]
    Closed,
}

/// A failed [`ActorRef::try_send`](crate::ActorRef::try_send) attempt.
///
/// The original message is retained and can be recovered with
/// [`into_message`](Self::into_message). Neither failure kind commits the
/// message, so retrying it elsewhere is safe.
#[derive(thiserror::Error)]
#[error("{kind}")]
pub struct TrySendError<M> {
    kind: TrySendErrorKind,
    message: M,
}

impl<M> TrySendError<M> {
    pub(crate) const fn new(kind: TrySendErrorKind, message: M) -> Self {
        Self { kind, message }
    }

    /// Returns why admission failed.
    pub const fn kind(&self) -> TrySendErrorKind {
        self.kind
    }

    /// Returns the message without retrying or dropping it.
    pub fn into_message(self) -> M {
        self.message
    }
}

impl<M> fmt::Debug for TrySendError<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrySendError")
            .field("kind", &self.kind)
            .field("message", &"<message>")
            .finish()
    }
}

/// The reason a synchronous admission attempt failed.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum TryCallErrorKind {
    /// No mailbox slot was immediately available.
    #[error("the actor mailbox is full")]
    Full,

    /// The actor no longer accepts new messages.
    #[error("the actor is closed to new messages")]
    Closed,
}

/// A failed [`ActorRef::try_call`](crate::ActorRef::try_call) attempt.
///
/// The original message is retained and can be recovered with
/// [`into_message`](Self::into_message). Neither failure kind commits the
/// message, so retrying it elsewhere is safe.
pub struct TryCallError<M> {
    kind: TryCallErrorKind,
    message: M,
}

impl<M> TryCallError<M> {
    pub(crate) const fn new(kind: TryCallErrorKind, message: M) -> Self {
        Self { kind, message }
    }

    /// Returns why admission failed.
    pub const fn kind(&self) -> TryCallErrorKind {
        self.kind
    }

    /// Returns the message without retrying or dropping it.
    pub fn into_message(self) -> M {
        self.message
    }
}

impl<M> fmt::Debug for TryCallError<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TryCallError")
            .field("kind", &self.kind)
            .field("message", &"<message>")
            .finish()
    }
}

impl<M> fmt::Display for TryCallError<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl<M> Error for TryCallError<M> {}
