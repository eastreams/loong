use std::{fmt, hash, sync::Arc};

use crate::{Actor, ActorRef};

/// A requested actor shutdown mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Shutdown {
    /// Finish dispatched replies, discard queued messages, then run cleanup.
    Stop,
    /// Dispatch the fixed accepted queue, finish its replies, then run cleanup.
    Drain,
    /// Drop the current cooperative future and queued messages without cleanup.
    Kill,
}

/// The result of submitting a shutdown request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ShutdownStatus {
    /// The request established or upgraded the actor's shutdown mode.
    Requested,
    /// A shutdown is already in progress.
    ///
    /// Stop and Drain are first-wins peers. Kill may upgrade either one.
    InProgress(Shutdown),
    /// The actor has already exited.
    Exited(ExitReason),
}

/// Why an actor terminated.
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
