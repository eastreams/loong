use std::fmt;

use slotmap::DefaultKey;

use crate::{Actor, ActorRef};

/// A requested actor shutdown mode.
///
/// Every mode closes admission as soon as the request commits. Stop and Drain
/// are graceful, first-wins peers; Kill may upgrade either one. Graceful
/// shutdown proceeds post-order through the owned actor tree. Kill skips actor
/// cleanup but still waits for descendants when possible. See [`ExitStatus`]
/// for the final local reason and subtree guarantee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Shutdown {
    /// Finishes dispatched replies and discards messages still queued for
    /// dispatch.
    ///
    /// Pending actor initialization finishes first.
    /// The actor then requests Stop from its children, waits for their terminal
    /// events, and runs [`Actor::on_stop`] with [`ExitReason::Stopped`].
    Stop,
    /// Dispatches the fixed queue accepted before Drain committed and finishes
    /// all resulting replies.
    ///
    /// Pending actor initialization finishes first.
    /// The actor then requests Drain from its children, waits for their terminal
    /// events, and runs [`Actor::on_stop`] with [`ExitReason::Drained`].
    Drain,
    /// Cancels cooperative actor work without running [`Actor::on_stop`].
    /// It waits for every retained child actor.
    ///
    /// This may prevent [`Actor::init`] or cancel it between polls.
    /// The runtime installs no actor value before initialization returns.
    ///
    /// Kill first drops interrupted initialization or an entered hook.
    /// This releases its mutable scope borrow.
    /// Kill then reaches children before active replies and queued messages are dropped.
    /// Descendants can begin termination before those destructors.
    /// The final status reports subtree confirmation.
    ///
    /// Kill takes effect between polls. It cannot interrupt a synchronous
    /// handler, a poll call that does not return, or user `Drop` code.
    Kill,
}

/// The result of submitting a shutdown request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownStatus {
    /// The request established or upgraded the actor's shutdown mode.
    ///
    /// This confirms only that the request committed, not that shutdown has
    /// completed.
    Requested,
    /// The request made no change because shutdown is already in progress.
    ///
    /// Stop or Drain identifies the graceful mode that already won. Kill either
    /// identifies an active Kill or reports that panic/executor teardown has made
    /// graceful shutdown impossible. Inspect the eventual [`ExitStatus`].
    InProgress(Shutdown),
    /// The actor has already published its terminal status.
    Exited(ExitStatus),
}

/// Why an actor terminated.
///
/// This value describes only the actor itself.
/// [`ExitStatus::subtree`] reports the runtime's descendant guarantee.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ExitReason {
    /// Stop completed, including graceful cleanup.
    Stopped,
    /// Drain completed, including graceful cleanup.
    Drained,
    /// Kill interrupted or discarded actor work.
    Killed,
    /// Actor code panicked and the panic was contained by the runtime.
    Panicked,
    /// The executor dropped this actor's task.
    Aborted,
}

impl fmt::Display for ExitReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Stopped => "stopped",
            Self::Drained => "drained",
            Self::Killed => "killed",
            Self::Panicked => "panicked",
            Self::Aborted => "aborted",
        };
        formatter.write_str(text)
    }
}

/// Whether the runtime confirmed every owned descendant terminated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SubtreeStatus {
    /// The runtime confirmed termination before status publication.
    Terminated,
    /// An abort path prevented termination confirmation.
    ///
    /// An aborted actor requests Kill from children it still owns.
    /// Its synchronous teardown cannot await their termination.
    /// `Unconfirmed` means positive proof is unavailable.
    /// It does not prove any descendant remains alive.
    /// This status does not stop a running parent.
    /// It remains unconfirmed through every ancestor.
    Unconfirmed,
}

/// The final outcome of one actor and its owned subtree.
///
/// [`reason`](Self::reason) describes only this actor.
/// [`subtree`](Self::subtree) reports the runtime's guarantee.
/// Descendant status never replaces the parent's local reason.
/// A parent may stop normally after a descendant aborts.
/// That produces `Stopped` with [`SubtreeStatus::Unconfirmed`].
/// `Panicked` may retain `Terminated` after asynchronous cleanup.
/// Local `Aborted` happens during synchronous task destruction.
/// It therefore always forces `Unconfirmed`.
///
/// The runtime publishes this value once.
/// Later lifecycle requests cannot change it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitStatus {
    reason: ExitReason,
    subtree: SubtreeStatus,
}

impl ExitStatus {
    pub(crate) const fn new(reason: ExitReason, subtree: SubtreeStatus) -> Self {
        let subtree = match reason {
            ExitReason::Aborted => SubtreeStatus::Unconfirmed,
            _ => subtree,
        };
        Self { reason, subtree }
    }

    /// Returns why this actor terminated.
    pub const fn reason(self) -> ExitReason {
        self.reason
    }

    /// Returns the runtime's subtree termination guarantee.
    pub const fn subtree(self) -> SubtreeStatus {
        self.subtree
    }
}

/// Opaque identity of one direct-child registration.
///
/// Compare identities only within one parent actor.
/// IDs from different parents may compare equal.
///
/// The generation rejects stale IDs when storage slots are reused. It can wrap
/// after 2^31 reuses of one slot. This is not a registry key.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ChildId(DefaultKey);

impl ChildId {
    pub(crate) const fn from_key(key: DefaultKey) -> Self {
        Self(key)
    }

    pub(crate) const fn key(self) -> DefaultKey {
        self.0
    }

    #[cfg(test)]
    /// Creates an invalid ID for tests that only schedule an event.
    pub(crate) fn invalid_for_test() -> Self {
        Self(DefaultKey::default())
    }
}

impl fmt::Debug for ChildId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildId(..)")
    }
}

/// The terminal event of a direct child actor.
///
/// While the parent is active, the runtime delivers this value serially to
/// [`Actor::on_child_exit`]. Hook entry and graceful cutoff share one lifecycle
/// gate: an event that loses the cutoff is absorbed, while a hook admitted first
/// is allowed to finish before parent cleanup proceeds.
/// Event equality is meaningful only within one direct parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildExit {
    child: ChildId,
    status: ExitStatus,
}

impl ChildExit {
    pub(crate) const fn new(child: ChildId, status: ExitStatus) -> Self {
        Self { child, status }
    }

    /// Returns the identity assigned by the direct parent.
    ///
    /// Compare it only with children spawned by that parent.
    pub const fn child(&self) -> &ChildId {
        &self.child
    }

    /// Returns the child actor's terminal status.
    pub const fn status(&self) -> ExitStatus {
        self.status
    }
}

/// A typed, non-owning reference to a child registered in its parent's tree.
///
/// Registration is complete when this value is returned.
/// Child initialization may still be pending.
/// The parent runtime retains lifecycle ownership. Cloning or dropping a
/// `Child` does not keep the child alive or initiate shutdown.
pub struct Child<A: Actor> {
    id: ChildId,
    actor_ref: ActorRef<A>,
}

impl<A: Actor> Child<A> {
    pub(crate) const fn new(id: ChildId, actor_ref: ActorRef<A>) -> Self {
        Self { id, actor_ref }
    }

    /// Returns the identity assigned by the direct parent.
    ///
    /// Compare it only with events observed by that parent.
    pub const fn id(&self) -> &ChildId {
        &self.id
    }

    /// Returns the child's message address.
    pub const fn actor_ref(&self) -> &ActorRef<A> {
        &self.actor_ref
    }

    /// Discards the identity wrapper and returns the message address.
    pub fn into_actor_ref(self) -> ActorRef<A> {
        self.actor_ref
    }
}

impl<A: Actor> Clone for Child<A> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            actor_ref: self.actor_ref.clone(),
        }
    }
}

impl<A: Actor> fmt::Debug for Child<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Child")
            .field("id", &self.id)
            .field("actor_ref", &self.actor_ref)
            .finish()
    }
}
