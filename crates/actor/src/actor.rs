use std::future::Future;

use crate::{ActorScope, ChildExit, ExitReason, IntoReply};

/// State that can be run as an actor.
///
/// Lifecycle hooks are awaited inside the actor's serial execution context.
/// While a hook is pending, the runtime does not dispatch handlers, poll active
/// replies, or enter another hook for this actor. Stop and Drain wait for an
/// entered hook to finish. Kill may drop it between polls; neither Kill nor
/// executor teardown can interrupt a poll call or user `Drop` code that does not
/// return.
///
/// Unlike a [`Handler`] reply, a hook future may retain its `&mut self` and
/// `&mut ActorScope<Self>` borrows across `await` for the method's `'a` lifetime;
/// the serial execution rule is what makes that exclusive borrow valid.
///
/// A panic from a hook is contained by the runtime and normally terminates the
/// actor with [`ExitReason::Panicked`]. A Kill that has already committed takes
/// precedence. Remaining children are killed before the parent's terminal event
/// is published.
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
        _scope: &'a mut ActorScope<Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Observes the terminal event of a direct child while the parent is active.
    ///
    /// The child has already terminated when this hook begins. A direct child
    /// contributes at most one event, and its exit reason does not by itself stop
    /// the parent. Events still pending when post-order parent cleanup begins,
    /// including exits caused by that cleanup, are absorbed by teardown instead
    /// of being re-entered as hooks.
    ///
    /// Restart policy is intentionally application-owned: the hook may spawn a
    /// replacement child, ignore the event, or request parent shutdown. While
    /// the parent is running, awaiting a call to the same parent from this hook
    /// cannot progress because the hook blocks dispatch. A hook panic fails the
    /// parent and kills its remaining children.
    fn on_child_exit<'a>(
        &'a mut self,
        _event: ChildExit,
        _scope: &'a mut ActorScope<Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        std::future::ready(())
    }

    /// Performs post-order cleanup for a successful Stop or Drain.
    ///
    /// The runtime enters this hook only after work retained by the selected mode
    /// has finished, queued work has been discarded where Stop requires it,
    /// child admission has closed, and every direct child has terminated. It
    /// requests the same graceful mode from remaining children, but a child may
    /// already be terminating for another reason. `reason` is therefore
    /// [`ExitReason::Stopped`] or [`ExitReason::Drained`], and attempts to spawn
    /// another child are rejected.
    ///
    /// Stop and Drain wait for this future. Kill may drop it between polls, in
    /// which case the final reason is [`ExitReason::Killed`]. A panic changes the
    /// final reason to [`ExitReason::Panicked`] unless Kill already committed.
    /// This hook is never entered for a prior Kill, panic, or executor
    /// cancellation.
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
    /// The typed value eventually returned to the caller.
    ///
    /// A reply may outlive synchronous handler dispatch and may cross a Tokio
    /// task boundary to its caller, so it must be owned, `Send`, and `'static`.
    type Reply: Send + 'static;
}

/// Handles one message type for an [`Actor`].
///
/// One actor may implement this trait for any number of message types. The
/// runtime erases each request only after pairing the concrete message with its
/// concrete reply channel, so no downcast is involved. Each implementation
/// statically selects one reply wrapper while keeping its concrete type opaque.
/// Mailbox FIFO determines the order in which eligible handlers are dispatched;
/// asynchronous replies may complete in a different order.
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
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, M> + use<Self, M>;
}
