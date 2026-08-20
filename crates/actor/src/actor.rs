use std::future::Future;

use crate::{
    ActorScope, ChildExit, ExitReason, StopScope,
    config::SupervisionConfig,
    reply::{IntoReply, ReplyExt},
    scheduling::{InterleavedScheduler, ReplyScheduler, SchedulerProfile},
    transport::{MessageConfig, MessageInbox, MessageSender, RuntimeInbox},
};

/// State that can be run as an actor.
///
/// Use [`#[actor(...)]`](macro@crate::actor) for built-in runtime profiles.
/// Manual configurations implement [`ActorConfig`](crate::ActorConfig).
/// [`MessageConfig`] supplies transport and reply scheduling.
/// [`SupervisionConfig`] supplies child actor ownership.
///
/// Initialization and lifecycle hooks run in the serial actor context.
/// While they are pending, handlers and actor-aware replies pause.
/// Another hook cannot enter for this actor.
/// Owned replies continue in independent Tokio tasks.
/// Stop and Drain wait for initialization and entered hooks.
/// Kill may cancel current serial work between polls.
/// Kill and executor teardown cannot interrupt a poll or user `Drop`.
///
/// Unlike a [`Handler`] reply, a hook may retain `&mut self` across `await`.
/// Initialization and child hooks may retain [`ActorScope`].
/// The stop hook may retain [`StopScope`].
/// Serial execution makes these borrows valid.
///
/// Initialization and hook panics are contained by the runtime.
/// A contained panic normally produces [`ExitReason::Panicked`].
/// A committed Kill instead produces [`ExitReason::Killed`].
/// Remaining children receive Kill before parent publication.
/// Their confirmation appears in
/// [`ExitStatus::subtree`](crate::ExitStatus::subtree).
pub trait Actor:
    MessageConfig<Inbox: RuntimeInbox<Self>, Scheduler: SchedulerProfile<Self>>
    + SupervisionConfig
    + Send
    + Sized
    + 'static
{
    /// Owned input used to construct this actor.
    type SpawnArgs: Send + 'static;

    /// Constructs the actor before its first handler dispatch.
    ///
    /// Initialization alone does not close mailbox admission.
    /// No handler runs before it returns `Self`.
    /// Stop and Drain close admission, then wait.
    /// Kill may prevent entry or cancel between polls.
    ///
    /// A self-call cannot progress during initialization.
    /// Initialization itself blocks every handler dispatch.
    /// A panic discards queued calls before dispatch.
    /// It also kills children and skips [`on_stop`](Self::on_stop).
    fn init<'a>(
        args: Self::SpawnArgs,
        scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = Self> + Send + 'a;

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
        event: ChildExit,
        scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        let _ = event;
        let _ = scope;
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
    /// use loac::{Actor, ActorScope, ExitReason, StopScope, actor};
    ///
    /// struct Parent;
    /// struct ChildActor;
    ///
    /// #[actor]
    /// impl Actor for ChildActor {
    ///     type SpawnArgs = Self;
    ///
    ///     async fn init(actor: Self, _scope: &mut ActorScope<'_, Self>) -> Self {
    ///         actor
    ///     }
    /// }
    ///
    /// #[actor(children = unbounded)]
    /// impl Actor for Parent {
    ///     type SpawnArgs = Self;
    ///
    ///     async fn init(actor: Self, _scope: &mut ActorScope<'_, Self>) -> Self {
    ///         actor
    ///     }
    ///
    ///     async fn on_stop(
    ///         &mut self,
    ///         _reason: ExitReason,
    ///         scope: &mut StopScope<'_, Self>,
    ///     ) {
    ///         scope.spawn_child::<ChildActor>(ChildActor);
    ///     }
    /// }
    /// ```
    fn on_stop<'a>(
        &'a mut self,
        reason: ExitReason,
        scope: &'a mut StopScope<'_, Self>,
    ) -> impl Future<Output = ()> + Send + 'a {
        let _ = reason;
        let _ = scope;
        std::future::ready(())
    }
}

/// An actor configured to own direct child actors.
///
/// [`#[actor(children)]`](macro@crate::actor) selects this capability automatically.
/// Fixed, dynamic, and unbounded limits all qualify.
/// The selected supervision profile provides this capability.
/// Do not implement this trait directly.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot spawn child actors",
    label = "enable `children` in this actor's configuration"
)]
pub trait HasChildren:
    Actor + SupervisionConfig<Children: crate::supervision::ChildSpawner>
{
}

// The active supervision profile proves this capability.
#[doc(hidden)]
#[diagnostic::do_not_recommend]
impl<A> HasChildren for A
where
    A: Actor,
    A::Children: crate::supervision::ChildSpawner,
{
}

/// An actor with public message transport operations.
///
/// [`#[actor(mailbox)]`](macro@crate::actor) selects this capability automatically.
/// Fixed, dynamic, and unbounded mailbox limits all qualify.
/// Actors without `mailbox` can still supervise children.
/// Their [`ActorRef`](crate::ActorRef) values retain lifecycle methods.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot receive messages",
    label = "select or implement a mailbox transport"
)]
pub trait HasMailbox:
    Actor
    + MessageConfig<
        Sender: MessageSender<Self>,
        Inbox: MessageInbox<Self>,
        Scheduler: ReplyScheduler<Self>,
    >
{
}

// Operations, not storage presence, prove mailbox capability.
#[doc(hidden)]
#[diagnostic::do_not_recommend]
impl<A> HasMailbox for A
where
    A: Actor,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
    A::Scheduler: ReplyScheduler<A>,
{
}

/// An actor configured to run interleaved replies.
///
/// [`#[actor(mailbox, interleaved)]`](macro@crate::actor) selects this capability.
/// Fixed, dynamic, and unbounded limits all qualify.
/// This capability also requires [`HasMailbox`].
/// It exposes [`InterleavedFutureExt::interleaved`](crate::InterleavedFutureExt::interleaved).
/// The selected scheduler provides this capability automatically.
/// Do not implement this trait directly.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot run interleaved replies",
    label = "interleaved replies require mailbox and interleaving capabilities"
)]
pub trait HasInterleaving:
    HasMailbox + MessageConfig<Scheduler: InterleavedScheduler<Self>>
{
}

// The active scheduler profile proves this capability.
#[doc(hidden)]
#[diagnostic::do_not_recommend]
impl<A> HasInterleaving for A
where
    A: HasMailbox,
    A::Scheduler: InterleavedScheduler<A>,
{
}

/// A typed request accepted by an actor.
///
/// Declare one with `#[derive(Message)]`.
/// The derive defaults its reply type to `()`.
/// Without `#[message(reply = Type)]` the message is send-only and does not
/// implement [`HasReply`], so it cannot be passed to [`crate::ActorRef::call`].
/// Add `#[message(reply = Type)]` to make it callable.
pub trait Message: Send + 'static {
    /// The typed value eventually returned to the caller.
    ///
    /// A reply may outlive synchronous handler dispatch and may cross a Tokio
    /// task boundary to its caller, so it must be `Send + 'static` and cannot
    /// borrow from the actor.
    type Reply: Send + 'static;
}

/// Marks a [`Message`] with an explicitly selected reply type.
///
/// `#[derive(Message)]` without `#[message(reply = Type)]` is send-only and
/// does not implement this trait, so [`crate::ActorRef::call`] and
/// [`crate::ActorRef::try_call`] are unavailable for it.
pub trait HasReply: Message {}

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
/// use loac::{Actor, ActorScope, Handler, Message, SyncHandler, actor};
///
/// struct Worker {
///     notifications: usize,
/// }
///
/// #[actor(mailbox)]
/// impl Actor for Worker {
///     type SpawnArgs = Self;
///
///     async fn init(actor: Self, _scope: &mut ActorScope<'_, Self>) -> Self {
///         actor
///     }
/// }
///
/// #[derive(Message)]
/// struct Notify;
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
pub trait SyncHandler<M: Message>: HasMailbox {
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
pub trait Handler<M: Message>: HasMailbox {
    /// Synchronously starts handling `message` and chooses its reply semantics.
    ///
    /// The runtime calls this method after the request commits to dispatch.
    /// Configured interleaved capacity must also permit dispatch.
    /// No exclusive reply may block actor work.
    /// The function runs to completion inside one actor
    /// turn: Kill cannot interrupt it, and abandoning the caller's response does
    /// not roll back effects that occur here. A panic is contained, fails this
    /// actor, and cancels its other active and queued work.
    ///
    /// The precise `use<Self, M>` capture list excludes the lifetimes of this
    /// invocation's `&mut self` and `scope` borrows. A returned asynchronous reply
    /// must therefore own everything it keeps across polls, such as a moved
    /// message or a cloned handle. It cannot carry either mutable borrow beyond
    /// this call. Use [`reply::Either`](crate::reply::Either) for runtime branching.
    /// Both branches must have statically known reply strategies.
    ///
    /// This helper compiles because the returned reply cannot capture either input
    /// borrow:
    ///
    /// ```
    /// use loac::{ActorScope, Handler, IntoReply, Message};
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
