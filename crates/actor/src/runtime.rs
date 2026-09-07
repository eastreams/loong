use std::{
    fmt,
    future::Future,
    ops::Deref,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::FutureExt;
use pin_project_lite::pin_project;

use crate::{
    Actor, ActorConfig, ActorRef, Child, ChildExit, ErasedFuture, ExitReason, ExitStatus,
    HasChildren, Shutdown, ShutdownStatus, SubtreeStatus,
    config::SupervisionConfig,
    mailbox::{ActorInbox, ActorInner, Control, HookEntryPermit, Mode},
    owned::OwnedTasks,
    scheduling::{ActorScheduler, RuntimeScheduler, SchedulerTurn, TurnContext},
    supervision::{
        ChildSpawner, ChildSupervisor,
        runtime::{ParentLink, RegisteredChild, RuntimeChildSpawner, RuntimeChildren, Seal},
    },
};

mod actor_loop;
mod scope;
mod shutdown;
mod task;

#[cfg(test)]
mod tests;

#[allow(
    unused_imports,
    reason = "grouped re-export keeps one import surface for runtime tests and sibling modules"
)]
pub(crate) use actor_loop::{DrainTurn, actor_turn, drain_turn, handle_child_exit, run_actor};
pub(crate) use scope::ScopeState;
pub use scope::{ActorScope, StopScope};
#[allow(
    unused_imports,
    reason = "grouped re-export keeps one import surface for runtime tests and sibling modules"
)]
pub(crate) use shutdown::{
    DiscardOutcome, TEARDOWN_DROP_BUDGET, close_and_discard, graceful_finish, kill_actor,
};
#[allow(
    unused_imports,
    reason = "grouped re-export keeps one import surface for runtime tests and sibling modules"
)]
pub(crate) use task::{
    ActorTask, ActorWorkGuard, ActorWorkState, ExitGuard, PreparedActor, Work, await_actor_work,
    start_child,
};

/// Actor-specific configuration applied to one spawn.
///
/// Dynamic mailbox options expose
/// [`with_mailbox_capacity`](crate::DynamicMailboxOptions::with_mailbox_capacity).
/// Dynamic interleaving options expose
/// [`with_max_in_flight`](crate::DynamicInterleavingOptions::with_max_in_flight).
/// Dynamic supervision options expose
/// [`with_max_children`](crate::DynamicChildrenOptions::with_max_children).
/// Pass changed options to [`spawn_with`].
/// Fixed and unbounded profiles expose no matching builder.
pub type SpawnOptions<A> = <A as ActorConfig>::Options;

/// Spawns a root actor with its default [`SpawnOptions`].
///
/// This schedules [`Actor::init`] and returns immediately.
/// Mailbox admission opens before initialization completes.
/// Calls wait for initialization before dispatch.
/// One-way sends only wait for admission.
///
/// The returned [`ActorOwner`] owns the actor lifecycle.
/// This function requires an active Tokio runtime.
#[must_use = "dropping the returned owner requests Kill"]
pub fn spawn<A: Actor>(args: A::SpawnArgs) -> ActorOwner<A> {
    spawn_with::<A>(args, SpawnOptions::<A>::default())
}

/// Spawns a root actor with explicit options.
///
/// Initialization and admission follow [`spawn`].
/// The returned [`ActorOwner`] owns the actor lifecycle.
/// This function requires an active Tokio runtime.
#[must_use = "dropping the returned owner requests Kill"]
pub fn spawn_with<A: Actor>(args: A::SpawnArgs, options: SpawnOptions<A>) -> ActorOwner<A> {
    ActorSpawner::<A>::with_options(options).spawn(args)
}

/// An address-first actor spawner.
///
/// Creating it opens the actor's mailbox and supervision state, so
/// [`actor_ref`](Self::actor_ref) is available before the actor itself is
/// constructed. Call [`spawn`](Self::spawn) with the actor's [`Actor::SpawnArgs`]
/// to construct and start it. Dropping an unstarted spawner requests `Kill` on
/// the preallocated address, so waiting callers observe [`CallError::Closed`](crate::CallError::Closed)
/// instead of hanging.
#[must_use = "dropping an unstarted spawner requests Kill on its address"]
pub struct ActorSpawner<A: Actor> {
    unstarted: Option<task::Unstarted<A>>,
    started: bool,
}

impl<A: Actor> ActorSpawner<A> {
    /// Creates an unstarted spawner with the actor's default [`SpawnOptions`].
    pub fn new() -> Self {
        Self::with_options(SpawnOptions::<A>::default())
    }

    /// Creates an unstarted spawner with explicit [`SpawnOptions`].
    pub fn with_options(options: SpawnOptions<A>) -> Self {
        Self {
            unstarted: Some(task::Unstarted::new(options)),
            started: false,
        }
    }

    /// Returns the actor's address before the actor is constructed or started.
    pub fn actor_ref(&self) -> ActorRef<A> {
        self.unstarted
            .as_ref()
            .expect("an actor spawner can only be started once")
            .actor_ref()
    }

    /// Constructs and starts the actor, returning its lifecycle owner.
    ///
    /// This consumes the spawner. The preallocated address remains valid; any
    /// calls admitted before this point wait for initialization as usual.
    pub fn spawn(mut self, args: A::SpawnArgs) -> ActorOwner<A> {
        self.started = true;
        let unstarted = self
            .unstarted
            .take()
            .expect("an actor spawner can only be started once");
        ActorOwner(unstarted.prepare(args).start_root())
    }
}

impl<A: Actor> Default for ActorSpawner<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Actor> Drop for ActorSpawner<A> {
    fn drop(&mut self) {
        if !self.started
            && let Some(unstarted) = &self.unstarted
        {
            let _ = unstarted.actor_ref().request_shutdown(Shutdown::Kill);
        }
    }
}

/// The unique lifecycle owner of a root actor.
///
/// This type is deliberately not cloneable. Dropping it requests a best-effort
/// Kill but cannot synchronously wait from `Drop`; use [`shutdown`](
/// Self::shutdown) or [`wait`](Self::wait) when confirmed normal-path subtree
/// termination matters. [`ExitStatus`] separates the actor's reason from its
/// subtree guarantee.
///
/// It derefs to [`ActorRef`], so address methods can be called directly on an
/// owner while it is still alive.
#[must_use = "dropping an actor owner requests Kill"]
pub struct ActorOwner<A: Actor>(ActorRef<A>);

impl<A: Actor> Deref for ActorOwner<A> {
    type Target = ActorRef<A>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<A: Actor> ActorOwner<A> {
    /// Returns a cloneable, non-owning address.
    #[must_use]
    pub fn actor_ref(&self) -> ActorRef<A> {
        self.0.clone()
    }

    /// Requests Stop, Drain, or Kill without waiting for completion.
    ///
    /// The request atomically closes admission if it establishes a mode. Stop
    /// and Drain are first-wins peers, while Kill may upgrade either one. See
    /// [`Shutdown`] for retained work and cleanup behavior, and
    /// [`ShutdownStatus`] for the meaning of the immediate result.
    #[must_use]
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.0.request_shutdown(shutdown)
    }

    /// Returns the terminal status if the actor has already exited.
    #[must_use]
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.0.exit_status()
    }

    /// Waits for the actor to publish its terminal event.
    ///
    /// The wait itself does not initiate shutdown. It borrows the owner
    /// exclusively, so no concurrent owner method can race the wait; after a
    /// timeout the caller can still request Kill.
    /// The status's reason describes only this actor.
    /// Its subtree status reports the runtime's termination guarantee.
    pub async fn wait(&mut self) -> ExitStatus {
        self.0.closed().await
    }

    /// Requests shutdown and waits for the terminal event described by
    /// [`wait`](Self::wait), including its subtree guarantee.
    ///
    /// The local reason can differ from the requested mode.
    /// Concurrent shutdown, panic, or executor teardown may win.
    ///
    /// This future owns the actor owner. Cancelling it before terminal
    /// publication therefore drops the owner and requests best-effort Kill. If
    /// Stop or Drain already committed, Drop upgrades it to Kill; if this future
    /// was never polled, the graceful request never committed. To retain control
    /// after cancelling a wait, call [`request_shutdown`](Self::request_shutdown)
    /// and apply the deadline to [`wait`](Self::wait) instead.
    pub async fn shutdown(mut self, shutdown: Shutdown) -> ExitStatus {
        let _ = self.request_shutdown(shutdown);
        self.wait().await
    }
}

impl<A: Actor> Drop for ActorOwner<A> {
    fn drop(&mut self) {
        let _ = self.0.request_shutdown(Shutdown::Kill);
    }
}

impl<A: Actor> fmt::Debug for ActorOwner<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOwner")
            .field("actor_ref", &self.0)
            .field("exit_status", &self.exit_status())
            .finish_non_exhaustive()
    }
}
