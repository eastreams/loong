use std::future::Future;

use crate::{ActorScope, ChildExit, ExitReason, IntoReply};

/// State that can be run as an actor.
///
/// Hooks run inside the actor's serial execution context. Only Kill may drop a
/// pending hook future.
pub trait Actor: Send + Sized + 'static {
    /// Runs once before the actor accepts its first handler dispatch.
    fn on_start<'a>(
        &'a mut self,
        _scope: &'a mut ActorScope<Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Observes the terminal event of a direct child while the parent is active.
    ///
    /// Restart policy is intentionally application-owned: the hook may spawn a
    /// replacement child, ignore the event, or request parent shutdown. Child
    /// exits caused by the parent's own shutdown are not re-entered as hooks.
    fn on_child_exit<'a>(
        &'a mut self,
        _event: ChildExit,
        _scope: &'a mut ActorScope<Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Performs graceful cleanup after this actor and all direct children have
    /// honored Stop or Drain.
    ///
    /// This hook is skipped for Kill, panic, and executor cancellation.
    fn on_stop<'a>(
        &'a mut self,
        _reason: ExitReason,
        _scope: &'a mut ActorScope<Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }
}

/// A typed request accepted by an actor.
pub trait Message: Send + 'static {
    /// The typed value returned to the caller.
    type Reply: Send + 'static;
}

/// Handles one message type for an [`Actor`].
///
/// One actor may implement this trait for any number of message types. The
/// runtime erases each request only after pairing the concrete message with its
/// concrete reply channel, so no downcast is involved. Each implementation
/// statically selects one reply wrapper while keeping its concrete type opaque.
pub trait Handler<M: Message>: Actor {
    /// Synchronously starts handling `message` and chooses its reply semantics.
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, M> + use<Self, M>;
}
