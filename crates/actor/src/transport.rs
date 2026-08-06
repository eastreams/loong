//! Advanced message transport extension points.
//!
//! Most actors should use [`#[actor(...)]`](macro@crate::actor).
//! Manual actors may implement these traits instead.
//! These traits are safe and unsealed.
//! Their behavioral contracts remain mandatory.
//! Violations cannot cause undefined behavior.
//! They can break lifecycle and delivery guarantees.
//!
//! # Conforming transport laws
//!
//! `open` returns paired endpoints for one empty transport.
//! Both endpoints remain open until runtime calls `close`.
//! Every successful enqueue stores exactly one carrier.
//! Its completion order defines mailbox FIFO.
//! The inbox returns each carrier exactly once.
//! It preserves FIFO while returning them.
//! `poll_recv(Pending)` registers the supplied Waker.
//! Progress or closure wakes that Waker.
//! `poll_recv(None)` requires closure and an empty inbox.
//! `try_recv` and `is_empty` must agree.
//! `close` is idempotent and preserves accepted carriers.
//! It permanently rejects new reservations.
//! `reserve_owned` is cancellation-safe.
//! Persistent waiters receive reservations in FIFO order.
//! Dropping any reservation releases its capacity.
//! `enqueue(Err)` only reports permanent closure.
//! It returns the unchanged carrier.
//! `enqueue` never invokes user code.
//! It never blocks, panics, or reenters Loong.
//! Inbox methods never panic.
//! Immediate operations never block.
//! `close` may wake reservation waiters.
//! No implementation drops an accepted carrier.
//! Inbox destruction retains no carrier.

use std::{
    future::Future,
    num::NonZeroUsize,
    task::{Context, Poll},
};

use tokio::sync::mpsc;

use crate::{Actor, ActorConfig, mailbox::Envelope};

/// Configures one actor's messaging state.
///
/// Implement this manually only for custom transports.
/// Built-in actors should use [`#[actor(...)]`](macro@crate::actor).
/// `open` creates one matched state triple.
/// Actor bounds reject incompatible state triples.
/// Actors without a mailbox use [`scheduling::Disabled`](crate::scheduling::Disabled).
/// Mailbox actors without interleaving use [`scheduling::Serial`](crate::scheduling::Serial).
pub trait MessageConfig: ActorConfig {
    /// Maximum mailbox dispatches during one actor lane visit.
    ///
    /// This applies while Running and Draining.
    /// It does not force a Tokio task yield.
    const MAILBOX_DISPATCH_BUDGET: NonZeroUsize =
        NonZeroUsize::new(16).expect("the default mailbox dispatch budget is nonzero");

    /// Shared sending state.
    type Sender: Send + Sync + 'static;

    /// Receiving state owned by the actor task.
    type Inbox: Send + 'static;

    /// Reply scheduling state paired with this transport.
    type Scheduler: Send + 'static;

    /// Opens sender, inbox, and scheduler together.
    ///
    /// Resolution finishes before the actor task starts.
    fn open(options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler);
}

/// An opaque accepted message owned by a transport.
///
/// A carrier can only move through transport operations.
/// Its message type remains private after erasure.
/// Transport code cannot dispatch its contents.
#[must_use = "dropping accepted work breaks the transport contract"]
pub struct ErasedEnvelope<A: Actor> {
    envelope: Box<dyn Envelope<A>>,
}

impl<A: Actor> ErasedEnvelope<A> {
    pub(crate) fn new<E>(envelope: Box<E>) -> Self
    where
        E: Envelope<A> + 'static,
    {
        Self { envelope }
    }

    pub(crate) fn into_envelope(self) -> Box<dyn Envelope<A>> {
        self.envelope
    }
}

/// Why an immediate reservation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TryReserveError {
    /// Capacity is temporarily exhausted.
    #[error("message transport capacity is full")]
    Full,
    /// The receiving side is permanently closed.
    #[error("message transport is closed")]
    Closed,
}

/// A reserved right to enqueue one carrier.
///
/// Dropping it must release reserved capacity.
/// Enqueue must not block, panic, or reenter Loong.
/// Success must retain exactly one carrier.
/// Failure must return the original carrier.
pub trait MessageReservation<A: Actor> {
    /// Transfers one carrier into its reserved slot.
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>>;
}

/// Shared message admission for one actor.
///
/// Successful enqueue order defines mailbox FIFO order.
pub trait MessageSender<A: Actor>: Send + Sync + 'static {
    /// A reservation borrowing this sender.
    type Reservation<'a>: MessageReservation<A> + 'a
    where
        Self: 'a;

    /// A reservation independent from this sender borrow.
    type OwnedReservation: MessageReservation<A> + Send + 'static;

    /// Attempts an immediate reservation without blocking.
    ///
    /// `Full` means temporary exhaustion.
    /// `Closed` means permanent closure.
    fn try_reserve(&self) -> Result<Self::Reservation<'_>, TryReserveError>;

    /// Waits for one owned reservation.
    ///
    /// `None` means permanent closure.
    /// This wait must be cancellation-safe.
    /// Persistent waiters must progress in FIFO order.
    fn reserve_owned(
        &self,
    ) -> impl Future<Output = Option<Self::OwnedReservation>> + Send + use<A, Self>;
}

/// Inbox operations required by every actor task.
///
/// `close` must permanently close new reservations.
/// Closing must preserve already accepted carriers.
/// `is_empty` must agree with `try_recv`.
/// Inbox destruction must not retain any carrier.
pub trait RuntimeInbox<A: Actor>: Send + 'static {
    /// Polls the next accepted carrier.
    ///
    /// `Ready(None)` means permanent closure.
    fn poll_recv(&mut self, task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>>;

    /// Removes one carrier without waiting.
    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>>;

    /// Returns whether no carrier is currently queued.
    fn is_empty(&self) -> bool;

    /// Permanently closes new reservations.
    fn close(&mut self);
}

/// An inbox that provides public message delivery.
///
/// This marker distinguishes mailboxes from runtime-only inboxes.
pub trait MessageInbox<A: Actor>: RuntimeInbox<A> {}

/// Sending storage for an actor without a mailbox.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSender;

impl NoSender {
    /// Opens transport storage without a mailbox.
    #[doc(hidden)]
    pub fn open() -> (Self, NoInbox) {
        (Self, NoInbox)
    }
}

/// Runtime inbox for an actor without a mailbox.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct NoInbox;

impl<A: Actor> RuntimeInbox<A> for NoInbox {
    fn poll_recv(&mut self, _task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        Poll::Pending
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>> {
        None
    }

    fn is_empty(&self) -> bool {
        true
    }

    fn close(&mut self) {}
}

/// Tokio bounded sending storage.
#[doc(hidden)]
pub struct BoundedSender<A: Actor>(mpsc::Sender<ErasedEnvelope<A>>);

impl<A: Actor> BoundedSender<A> {
    /// Opens one bounded transport.
    #[doc(hidden)]
    pub fn open(capacity: NonZeroUsize) -> (Self, BoundedInbox<A>) {
        let (sender, inbox) = mpsc::channel(capacity.get());
        (Self(sender), BoundedInbox(inbox))
    }
}

/// Tokio bounded receiving storage.
#[doc(hidden)]
pub struct BoundedInbox<A: Actor>(mpsc::Receiver<ErasedEnvelope<A>>);

/// A borrowed Tokio bounded reservation.
#[doc(hidden)]
pub struct BoundedReservation<'a, A: Actor>(mpsc::Permit<'a, ErasedEnvelope<A>>);

/// An owned Tokio bounded reservation.
#[doc(hidden)]
pub struct OwnedBoundedReservation<A: Actor>(mpsc::OwnedPermit<ErasedEnvelope<A>>);

impl<A: Actor> MessageReservation<A> for BoundedReservation<'_, A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        self.0.send(envelope);
        Ok(())
    }
}

impl<A: Actor> MessageReservation<A> for OwnedBoundedReservation<A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        drop(self.0.send(envelope));
        Ok(())
    }
}

impl<A: Actor> MessageSender<A> for BoundedSender<A> {
    type Reservation<'a>
        = BoundedReservation<'a, A>
    where
        Self: 'a;
    type OwnedReservation = OwnedBoundedReservation<A>;

    fn try_reserve(&self) -> Result<Self::Reservation<'_>, TryReserveError> {
        self.0
            .try_reserve()
            .map(BoundedReservation)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(()) => TryReserveError::Full,
                mpsc::error::TrySendError::Closed(()) => TryReserveError::Closed,
            })
    }

    fn reserve_owned(
        &self,
    ) -> impl Future<Output = Option<Self::OwnedReservation>> + Send + use<A> {
        let sender = self.0.clone();
        async move {
            sender
                .reserve_owned()
                .await
                .ok()
                .map(OwnedBoundedReservation)
        }
    }
}

impl<A: Actor> RuntimeInbox<A> for BoundedInbox<A> {
    fn poll_recv(&mut self, task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        self.0.poll_recv(task)
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>> {
        self.0.try_recv().ok()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn close(&mut self) {
        self.0.close();
    }
}

impl<A: Actor> MessageInbox<A> for BoundedInbox<A> {}

/// Tokio unbounded sending storage.
#[doc(hidden)]
pub struct UnboundedSender<A: Actor>(mpsc::UnboundedSender<ErasedEnvelope<A>>);

impl<A: Actor> UnboundedSender<A> {
    /// Opens one unbounded transport.
    #[doc(hidden)]
    pub fn open() -> (Self, UnboundedInbox<A>) {
        let (sender, inbox) = mpsc::unbounded_channel();
        (Self(sender), UnboundedInbox(inbox))
    }
}

impl<A: Actor> Clone for UnboundedSender<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Tokio unbounded receiving storage.
#[doc(hidden)]
pub struct UnboundedInbox<A: Actor>(mpsc::UnboundedReceiver<ErasedEnvelope<A>>);

impl<A: Actor> MessageReservation<A> for &UnboundedSender<A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        self.0.send(envelope).map_err(|error| error.0)
    }
}

impl<A: Actor> MessageReservation<A> for UnboundedSender<A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        self.0.send(envelope).map_err(|error| error.0)
    }
}

impl<A: Actor> MessageSender<A> for UnboundedSender<A> {
    type Reservation<'a>
        = &'a Self
    where
        Self: 'a;
    type OwnedReservation = Self;

    fn try_reserve(&self) -> Result<Self::Reservation<'_>, TryReserveError> {
        if self.0.is_closed() {
            Err(TryReserveError::Closed)
        } else {
            Ok(self)
        }
    }

    fn reserve_owned(
        &self,
    ) -> impl Future<Output = Option<Self::OwnedReservation>> + Send + use<A> {
        std::future::ready((!self.0.is_closed()).then(|| self.clone()))
    }
}

impl<A: Actor> RuntimeInbox<A> for UnboundedInbox<A> {
    fn poll_recv(&mut self, task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        self.0.poll_recv(task)
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>> {
        self.0.try_recv().ok()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn close(&mut self) {
        self.0.close();
    }
}

impl<A: Actor> MessageInbox<A> for UnboundedInbox<A> {}

#[cfg(test)]
mod tests;
