use std::future::Future;

use crate::{
    ActorScope, ChildExit, ExitReason, StopScope,
    reply::{IntoReply, ReplyExt},
};

/// State that can be run as an actor.
///
/// Lifecycle hooks are awaited inside the actor's serial execution context.
/// While a hook is pending, handlers and actor-aware replies pause.
/// Another hook cannot enter for this actor.
/// Owned replies continue in independent Tokio tasks.
/// Stop and Drain wait for an entered hook.
/// Kill may drop it between polls.
/// Kill and executor teardown cannot interrupt a poll or user `Drop`.
///
/// Unlike a [`Handler`] reply, a hook may retain `&mut self` across `await`.
/// Startup and child hooks may also retain [`ActorScope`].
/// The stop hook may retain [`StopScope`].
/// Serial execution makes these borrows valid.
///
/// A hook panic is contained by the runtime.
/// It normally produces [`ExitReason::Panicked`].
/// A committed Kill instead produces [`ExitReason::Killed`].
/// Remaining children receive Kill before parent publication.
/// Their confirmation appears in
/// [`ExitStatus::subtree`](crate::ExitStatus::subtree).
pub trait Actor: Send + Sized + 'static {
    /// Runs once before the actor performs its first handler dispatch.
    ///
    /// Calls may enter the bounded mailbox while this future is pending, but no
    /// handler runs until it completes. Stop and Drain close admission and wait
    /// for startup to finish; Kill may cancel startup between polls.
    ///
    /// Awaiting a call to this same actor cannot make progress because startup
    /// itself blocks dispatch. If this hook panics, queued calls are discarded
    /// with [`crate::CallError::BeforeDispatch`], children are killed, and
    /// [`on_stop`](Self::on_stop) is skipped.
    fn on_start<'a>(
        &'a mut self,
        _scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Observes the terminal event of a direct child while the parent is active.
    ///
    /// The child has already terminated when this hook begins. A direct child
    /// contributes at most one event. Its local reason does not stop the parent.
    /// An unconfirmed subtree remains sticky for the parent.
    /// Hook entry is linearized with Stop and Drain: an entry that
    /// commits first is allowed to finish before graceful shutdown proceeds,
    /// while an event whose hook loses that cutoff is absorbed without calling
    /// user code. Kill may still cancel an entered hook between polls.
    ///
    /// Restart policy is intentionally application-owned: the hook may spawn a
    /// replacement child, ignore the event, or request parent shutdown. While
    /// the parent is running, awaiting a call to the same parent from this hook
    /// cannot progress because the hook blocks dispatch. A hook panic fails the
    /// parent and kills its remaining children.
    fn on_child_exit<'a>(
        &'a mut self,
        _event: ChildExit,
        _scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Performs post-order cleanup for a successful Stop or Drain.
    ///
    /// The runtime enters this hook only after work retained by the selected mode
    /// has finished, queued work has been discarded where Stop requires it,
    /// child spawning has ended, and every direct child has terminated. It
    /// requests the same graceful mode from remaining children, but a child may
    /// already be terminating for another reason. `reason` is therefore
    /// [`ExitReason::Stopped`] or [`ExitReason::Drained`]. [`StopScope`] cannot
    /// change child topology.
    ///
    /// An unconfirmed child subtree does not skip this hook.
    /// The parent keeps its local graceful reason.
    /// Its final subtree becomes
    /// [`SubtreeStatus::Unconfirmed`](crate::SubtreeStatus::Unconfirmed).
    ///
    /// Stop and Drain wait for this future. Kill may drop it between polls.
    /// Kill then sets the local reason to [`ExitReason::Killed`].
    /// A panic sets it to [`ExitReason::Panicked`].
    /// An earlier Kill keeps precedence.
    /// This hook never runs after Kill, panic, or executor cancellation.
    ///
    /// Child spawning is unavailable during cleanup:
    ///
    /// ```compile_fail
    /// use loong_actor::{Actor, ExitReason, StopScope};
    ///
    /// struct Parent;
    /// struct ChildActor;
    ///
    /// impl Actor for ChildActor {}
    ///
    /// impl Actor for Parent {
    ///     async fn on_stop(
    ///         &mut self,
    ///         _reason: ExitReason,
    ///         scope: &mut StopScope<'_, Self>,
    ///     ) {
    ///         scope.spawn_child(ChildActor);
    ///     }
    /// }
    /// ```
    fn on_stop<'a>(
        &'a mut self,
        _reason: ExitReason,
        _scope: &'a mut StopScope<'_, Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }
}

/// A typed request accepted by an actor.
pub trait Message: Send + 'static {
    /// The typed value eventually returned to the caller.
    ///
    /// A reply may outlive synchronous handler dispatch and may cross a Tokio
    /// task boundary to its caller, so it must be owned, `Send`, and `'static`.
    type Reply: Send + 'static;
}

/// Handles one message by producing its reply before returning.
///
/// Use this trait when no asynchronous reply work remains. The runtime adapts
/// it to [`Handler`] and submits the returned value with
/// [`ReplyExt::ready`](crate::ReplyExt::ready).
///
/// `Sync` describes reply production. It does not permit blocking the actor
/// task. The method runs inside the synchronous dispatch turn. It inherits the
/// panic and Kill behavior documented by [`Handler::handle`].
///
/// Implement either `SyncHandler<M>` or `Handler<M>` for one actor and message
/// pair. The blanket adaptation makes implementing both a conflicting impl.
///
/// A unit reply needs no explicit return expression:
///
/// ```
/// use loong_actor::{Actor, ActorScope, Handler, Message, SyncHandler};
///
/// struct Worker {
///     notifications: usize,
/// }
///
/// impl Actor for Worker {}
///
/// struct Notify;
///
/// impl Message for Notify {
///     type Reply = ();
/// }
///
/// impl SyncHandler<Notify> for Worker {
///     fn handle(&mut self, _message: Notify, _scope: &mut ActorScope<'_, Self>) {
///         self.notifications += 1;
///     }
/// }
///
/// fn accepts_handler<A: Handler<Notify>>() {}
/// accepts_handler::<Worker>();
/// ```
pub trait SyncHandler<M: Message>: Actor {
    /// Processes `message` and returns its completed reply value.
    fn handle(&mut self, message: M, scope: &mut ActorScope<'_, Self>) -> M::Reply;
}

/// Handles one message type for an [`Actor`].
///
/// Use [`SyncHandler`] when `handle` produces the reply before returning.
///
/// One actor may implement this trait for any number of message types. The
/// runtime erases each request only after pairing the concrete message with its
/// concrete reply channel, so no downcast is involved. Each implementation
/// statically selects one concrete reply representation while keeping its type
/// opaque. Mailbox FIFO determines the order in which eligible handlers are
/// dispatched; asynchronous replies may complete in a different order.
pub trait Handler<M: Message>: Actor {
    /// Synchronously starts handling `message` and chooses its reply semantics.
    ///
    /// The runtime calls this method only after the request has committed to the
    /// dispatch phase, an in-flight slot is available, and no exclusive reply is
    /// blocking actor work. The function runs to completion inside one actor
    /// turn: Kill cannot interrupt it, and abandoning the caller's response does
    /// not roll back effects that occur here. A panic is contained, fails this
    /// actor, and cancels its other active and queued work.
    ///
    /// The precise `use<Self, M>` capture list excludes the lifetimes of this
    /// invocation's `&mut self` and `scope` borrows. A returned asynchronous reply
    /// must therefore own everything it keeps across polls, such as a moved
    /// message or a cloned handle; it cannot carry either mutable borrow beyond
    /// this call. Use [`crate::reply::Either`] when runtime branching requires two
    /// statically known reply strategies.
    ///
    /// This helper compiles because the returned reply cannot capture either input
    /// borrow:
    ///
    /// ```
    /// use loong_actor::{ActorScope, Handler, IntoReply, Message};
    ///
    /// fn detach_reply<A, M>(
    ///     actor: &mut A,
    ///     message: M,
    ///     scope: &mut ActorScope<'_, A>,
    /// ) -> impl IntoReply<A, M> + use<A, M>
    /// where
    ///     A: Handler<M>,
    ///     M: Message,
    /// {
    ///     actor.handle(message, scope)
    /// }
    /// ```
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<Self, M>;
}

impl<A, M> Handler<M> for A
where
    A: SyncHandler<M>,
    M: Message,
{
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<A, M> {
        <A as SyncHandler<M>>::handle(self, message, scope).ready()
    }
}
