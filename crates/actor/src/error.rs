use std::{error::Error, fmt};

use thiserror::Error;

use crate::ExitReason;

/// A failure after or while attempting asynchronous admission.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CallError {
    /// The actor stopped accepting requests before this message was committed.
    #[error("the actor is closed to new messages")]
    Closed,

    /// The message was accepted, but its handler was never invoked.
    #[error("the actor exited before dispatch: {0}")]
    BeforeDispatch(ExitReason),

    /// The handler began and was interrupted before producing its reply.
    ///
    /// External effects may already have happened, so this error is not proof
    /// that retrying the request is safe.
    #[error("the actor exited during dispatch: {0}")]
    DuringDispatch(ExitReason),

    /// The response channel vanished without the runtime reporting a phase.
    #[error("the actor response channel was lost")]
    ResponseLost,
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

/// A child actor rejected because its parent entered lifecycle cleanup.
///
/// The actor value was never spawned and can be recovered with
/// [`into_actor`](Self::into_actor).
#[derive(thiserror::Error)]
#[error("the parent no longer accepts child actors")]
pub struct SpawnChildError<A> {
    actor: A,
}

impl<A> SpawnChildError<A> {
    pub(crate) const fn new(actor: A) -> Self {
        Self { actor }
    }

    /// Returns the actor value that was not spawned.
    pub fn into_actor(self) -> A {
        self.actor
    }
}

impl<A> fmt::Debug for SpawnChildError<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SpawnChildError")
            .field("actor", &"<actor>")
            .finish()
    }
}
