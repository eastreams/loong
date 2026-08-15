use std::{
    convert::Infallible,
    future::Future,
    sync::Arc,
    task::{Context, Poll},
};

use slotmap::{DefaultKey, SlotMap};
use tokio::sync::mpsc;

use crate::{
    Actor, ActorRef, ChildExit, ChildId, ExitReason, ExitStatus, Shutdown, SubtreeStatus,
    mailbox::{ActorInner, Control},
    runtime::PreparedActor,
};

use super::{ChildSpawner, Disabled, Dynamic, Fixed, Full, Unbounded};

#[cfg(test)]
pub(crate) mod tests;

/// Grants access to sealed supervision runtime bridges.
pub struct Seal;

/// Runtime operations required by every supervision profile.
#[doc(hidden)]
pub trait RuntimeChildren: Send + 'static {
    fn poll_exit(&mut self, task: &mut Context<'_>) -> Poll<ChildExit>;

    fn reap(&mut self, event: &ChildExit) -> bool;

    fn request_all(&self, shutdown: Shutdown);

    fn wait_all(&mut self) -> impl Future<Output = ()> + Send;

    fn terminal_status(&self, reason: ExitReason) -> ExitStatus;
}

/// Runtime operations available only to enabled profiles.
#[doc(hidden)]
#[allow(
    private_bounds,
    reason = "child storage remains private to this module"
)]
pub trait RuntimeChildSpawner<P>: RuntimeChildren + ChildProfile
where
    P: ChildSpawner + ?Sized,
{
    fn admit<T>(&self, value: T) -> Result<T, P::Error<T>>;

    fn register<A: Actor>(&mut self, prepared: PreparedActor<A>) -> RegisteredChild<A> {
        self.state_mut().register(prepared)
    }
}

pub(crate) struct ParentLink {
    id: ChildId,
    events: mpsc::UnboundedSender<ChildExit>,
}

impl ParentLink {
    fn new(id: ChildId, events: mpsc::UnboundedSender<ChildExit>) -> Self {
        Self { id, events }
    }

    pub(crate) fn publish(&self, status: ExitStatus) {
        let _ = self.events.send(ChildExit::new(self.id, status));
    }
}

/// A registered child actor whose task has not started.
/// This token is the only child task-start capability.
pub struct RegisteredChild<A: Actor> {
    parent: ParentLink,
    prepared: PreparedActor<A>,
}

impl<A: Actor> RegisteredChild<A> {
    pub(crate) fn into_parts(self) -> (PreparedActor<A>, ParentLink, ChildId) {
        let id = self.parent.id;
        (self.prepared, self.parent, id)
    }
}

/// Cold ownership operations for heterogeneous child storage.
///
/// Each object shares its typed [`ActorRef`] allocation.
trait ErasedActor: Send + Sync {
    fn control(&self) -> &Control;
}

impl<A: Actor> ErasedActor for ActorInner<A> {
    fn control(&self) -> &Control {
        &self.control
    }
}

pub(crate) struct ErasedActorOwner(Arc<dyn ErasedActor>);

impl ErasedActorOwner {
    fn new<A: Actor>(actor_ref: &ActorRef<A>) -> Self {
        Self(Arc::clone(&actor_ref.0) as Arc<dyn ErasedActor>)
    }

    fn control(&self) -> &Control {
        self.0.control()
    }

    async fn wait(&self) -> ExitStatus {
        self.control().wait_for_exit().await
    }
}

impl Drop for ErasedActorOwner {
    fn drop(&mut self) {
        self.control().request(Shutdown::Kill);
    }
}

/// Owns enabled direct-child registrations and terminal events.
pub struct ChildSet {
    actors: SlotMap<DefaultKey, ErasedActorOwner>,
    // Removed children cannot erase a lost subtree guarantee.
    subtree: SubtreeStatus,
    // Child teardown never waits for parent work.
    // Owning both endpoints makes early closure invalid.
    event_tx: mpsc::UnboundedSender<ChildExit>,
    event_rx: mpsc::UnboundedReceiver<ChildExit>,
}

impl ChildSet {
    pub(super) fn new() -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            actors: SlotMap::new(),
            subtree: SubtreeStatus::Terminated,
            event_tx,
            event_rx,
        }
    }

    fn len(&self) -> usize {
        self.actors.len()
    }

    /// Registers ownership without starting the child task.
    fn register<A: Actor>(&mut self, prepared: PreparedActor<A>) -> RegisteredChild<A> {
        let child_ref = prepared.actor_ref();
        let owner = ErasedActorOwner::new(&child_ref);
        let key = self.actors.insert(owner);
        let id = ChildId::from_key(key);
        let parent = ParentLink::new(id, self.event_tx.clone());
        RegisteredChild { parent, prepared }
    }

    fn poll_exit(&mut self, task: &mut Context<'_>) -> Poll<ChildExit> {
        match self.event_rx.poll_recv(task) {
            Poll::Ready(Some(event)) => Poll::Ready(event),
            Poll::Ready(None) => {
                unreachable!("child-exit receiver closed while parent runtime was alive")
            }
            Poll::Pending => Poll::Pending,
        }
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, actor: ErasedActorOwner) -> ChildId {
        ChildId::from_key(self.actors.insert(actor))
    }

    fn reap(&mut self, event: &ChildExit) -> bool {
        if self.actors.remove(event.child().key()).is_none() {
            return false;
        }
        if event.status().subtree() == SubtreeStatus::Unconfirmed {
            self.subtree = SubtreeStatus::Unconfirmed;
        }
        true
    }

    fn request_all(&self, shutdown: Shutdown) {
        for actor in self.actors.values() {
            actor.control().request(shutdown);
        }
    }

    async fn wait_all(&mut self) {
        // Persist each result before another cancellation point.
        let Self {
            actors, subtree, ..
        } = self;
        for actor in actors.values() {
            if actor.wait().await.subtree() == SubtreeStatus::Unconfirmed {
                *subtree = SubtreeStatus::Unconfirmed;
            }
        }
        actors.clear();
    }

    fn terminal_status(&self, reason: ExitReason) -> ExitStatus {
        ExitStatus::new(reason, self.subtree)
    }
}

#[doc(hidden)]
pub trait ChildProfile: Send + 'static {
    fn state(&self) -> &ChildSet;

    fn state_mut(&mut self) -> &mut ChildSet;
}

impl<const N: usize> ChildProfile for Fixed<N> {
    fn state(&self) -> &ChildSet {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ChildSet {
        &mut self.state
    }
}

impl ChildProfile for Dynamic {
    fn state(&self) -> &ChildSet {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ChildSet {
        &mut self.state
    }
}

impl ChildProfile for Unbounded {
    fn state(&self) -> &ChildSet {
        &self.state
    }

    fn state_mut(&mut self) -> &mut ChildSet {
        &mut self.state
    }
}

impl<P: ChildProfile> RuntimeChildren for P {
    fn poll_exit(&mut self, task: &mut Context<'_>) -> Poll<ChildExit> {
        self.state_mut().poll_exit(task)
    }

    fn reap(&mut self, event: &ChildExit) -> bool {
        self.state_mut().reap(event)
    }

    fn request_all(&self, shutdown: Shutdown) {
        self.state().request_all(shutdown);
    }

    fn wait_all(&mut self) -> impl Future<Output = ()> + Send {
        self.state_mut().wait_all()
    }

    fn terminal_status(&self, reason: ExitReason) -> ExitStatus {
        self.state().terminal_status(reason)
    }
}

impl RuntimeChildren for Disabled {
    fn poll_exit(&mut self, _task: &mut Context<'_>) -> Poll<ChildExit> {
        Poll::Pending
    }

    fn reap(&mut self, _event: &ChildExit) -> bool {
        false
    }

    fn request_all(&self, _shutdown: Shutdown) {}

    fn wait_all(&mut self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }

    fn terminal_status(&self, reason: ExitReason) -> ExitStatus {
        ExitStatus::new(reason, SubtreeStatus::Terminated)
    }
}

impl<const N: usize> RuntimeChildSpawner<Fixed<N>> for Fixed<N> {
    fn admit<T>(&self, value: T) -> Result<T, Full<T>> {
        if self.state.len() < N {
            Ok(value)
        } else {
            Err(Full::new(value))
        }
    }
}

impl RuntimeChildSpawner<Dynamic> for Dynamic {
    fn admit<T>(&self, value: T) -> Result<T, Full<T>> {
        if self.state.len() < self.limit.get() {
            Ok(value)
        } else {
            Err(Full::new(value))
        }
    }
}

impl RuntimeChildSpawner<Unbounded> for Unbounded {
    fn admit<T>(&self, value: T) -> Result<T, Infallible> {
        Ok(value)
    }
}
