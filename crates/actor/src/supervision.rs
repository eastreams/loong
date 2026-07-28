use std::{fmt, hash, sync::Arc};

use crate::{Actor, ActorRef};

/// A requested actor shutdown mode.
///
/// Every mode closes admission as soon as the request commits. Stop and Drain
/// are graceful, first-wins peers; Kill may upgrade either one. Graceful
/// shutdown proceeds post-order through the owned actor tree, while Kill skips
/// actor cleanup but still waits for descendants before publishing a normal
/// terminal event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Shutdown {
    /// Finishes dispatched replies and discards messages still queued for
    /// dispatch.
    ///
    /// The actor then requests Stop from its children, waits for their terminal
    /// events, and runs [`Actor::on_stop`] with [`ExitReason::Stopped`].
    Stop,
    /// Dispatches the fixed queue accepted before Drain committed and finishes
    /// all resulting replies.
    ///
    /// The actor then requests Drain from its children, waits for their terminal
    /// events, and runs [`Actor::on_stop`] with [`ExitReason::Drained`].
    Drain,
    /// Cancels cooperative actor work without running [`Actor::on_stop`] and
    /// waits for the owned subtree to terminate.
    ///
    /// If Kill interrupts an entered lifecycle hook, that hook is first dropped
    /// to release its mutable scope borrow. Once the scope is available, Kill is
    /// submitted to children before active replies and queued messages are
    /// dropped, allowing descendants to begin termination ahead of arbitrary
    /// user destructors. The terminal event is published only after the subtree
    /// exits.
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
    /// graceful shutdown impossible; inspect the eventual [`ExitReason`] for the
    /// terminal guarantee.
    InProgress(Shutdown),
    /// The actor has already published its terminal reason.
    Exited(ExitReason),
}

/// Why an actor terminated.
///
/// Stopped, Drained, Killed, and Panicked are strong terminal events: the
/// actor's owned descendants have terminated before the reason is published.
/// Aborted is deliberately weaker because synchronous executor teardown cannot
/// wait for descendants.
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
    /// The executor dropped the actor task outside its normal lifecycle.
    ///
    /// Descendant Kill has been initiated, but asynchronous subtree termination
    /// cannot be confirmed from the task's synchronous drop path.
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

#[derive(Debug)]
struct ChildIdentity {
    _private: u8,
}

/// Opaque, allocation-backed identity of one direct child.
///
/// Identity is local to the runtime tree; it is not a registry key or a
/// process-wide scalar identifier.
#[derive(Clone)]
pub struct ChildId(Arc<ChildIdentity>);

impl ChildId {
    pub(crate) fn new() -> Self {
        Self(Arc::new(ChildIdentity { _private: 0 }))
    }
}

impl fmt::Debug for ChildId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildId(..)")
    }
}

impl PartialEq for ChildId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ChildId {}

impl hash::Hash for ChildId {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

/// The terminal event of a direct child actor.
///
/// While the parent is active, the runtime delivers this value serially to
/// [`Actor::on_child_exit`]. Hook entry and graceful cutoff share one lifecycle
/// gate: an event that loses the cutoff is absorbed, while a hook admitted first
/// is allowed to finish before parent cleanup proceeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChildExit {
    child: ChildId,
    reason: ExitReason,
}

impl ChildExit {
    pub(crate) const fn new(child: ChildId, reason: ExitReason) -> Self {
        Self { child, reason }
    }

    /// Returns the child that exited.
    pub const fn child(&self) -> &ChildId {
        &self.child
    }

    /// Returns the child's terminal reason.
    pub const fn reason(&self) -> ExitReason {
        self.reason
    }
}

/// A typed, non-owning reference to a child registered in its parent's tree.
///
/// The parent [`crate::ActorScope`] retains lifecycle ownership. Cloning or
/// dropping a `Child` does not keep the child alive or initiate shutdown.
pub struct Child<A: Actor> {
    id: ChildId,
    actor_ref: ActorRef<A>,
}

impl<A: Actor> Child<A> {
    pub(crate) const fn new(id: ChildId, actor_ref: ActorRef<A>) -> Self {
        Self { id, actor_ref }
    }

    /// Returns the child's tree identity.
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
            id: self.id.clone(),
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
