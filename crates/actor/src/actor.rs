use std::{future::Future, pin::Pin};

use tokio::sync::{mpsc, oneshot};

use crate::{
    ActorScope, ChildExit, ExitReason, Shutdown, StopScope, StreamOut, Writer,
    access::Cx,
    config::SupervisionConfig,
    reply::{
        CxReply, CxStream, IntoReply, IntoStreamReply, RawKind, RawStreamKind, StreamDispatch,
        StreamKind, StreamMessage, SyncKind,
    },
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

    /// Creates an address-first spawner for this actor.
    ///
    /// The spawner opens this actor's mailbox and supervision state before
    /// [`SpawnArgs`](Self::SpawnArgs) exist, so callers can obtain the
    /// [`ActorRef`](crate::ActorRef) first and pass it to other actors before
    /// constructing this one. Start it with
    /// [`ActorSpawner::spawn`](crate::ActorSpawner::spawn).
    fn spawner() -> crate::ActorSpawner<Self>
    where
        Self: Sized,
    {
        crate::ActorSpawner::<Self>::new()
    }

    /// Creates an address-first spawner with explicit [`SpawnOptions`](crate::SpawnOptions).
    ///
    /// This is the [`spawner`](Self::spawner) counterpart to
    /// [`spawn_with`](crate::spawn_with).
    fn spawner_with(options: crate::SpawnOptions<Self>) -> crate::ActorSpawner<Self>
    where
        Self: Sized,
    {
        crate::ActorSpawner::<Self>::with_options(options)
    }

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

    /// Observes that Stop or Drain has committed, before replies drain.
    ///
    /// The runtime calls this hook synchronously in the actor task exactly
    /// once, when the actor loop observes the graceful shutdown mode. It runs
    /// before already-dispatched replies are drained, so the actor can cancel
    /// background loops or signal them to exit at their next turn boundary.
    ///
    /// Kill, panic, abort, and executor cancellation never call this hook. A
    /// panic inside the hook fails the actor.
    fn on_shutdown(&mut self, _shutdown: Shutdown) {}

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
/// Declare one with `#[derive(Message)]`. The derive supports four shapes:
///
/// - `#[message(reply = Type)]` makes an ordinary callable message handled by
///   [`crate::Handler`].
/// - `#[message(stream = Item, reply = Final)]` makes a stream message handled
///   by [`crate::StreamHandler`].
/// - `#[message(raw = Type)]` makes an ordinary message handled by
///   [`crate::RawHandler`].
/// - `#[message(raw_stream = Item, reply = Final)]` makes a stream message
///   handled by [`crate::RawStreamHandler`].
///
/// The reply type defaults to `()` whenever the attribute omits it. Without
/// `reply` the message does not implement [`HasReply`], so it cannot be passed
/// to [`crate::ActorRef::call`]. The `stream` shapes make
/// [`crate::ActorRef::call`] return [`crate::reply::StreamReply`].
/// See the derive macro documentation for the full attribute syntax.
pub trait Message: Send + 'static {
    /// The typed value eventually returned to the caller.
    ///
    /// A reply may outlive synchronous handler dispatch and may cross a Tokio
    /// task boundary to its caller, so it must be `Send + 'static` and cannot
    /// borrow from the actor.
    type Reply: Send + 'static;

    /// The reply channel shape this message selects.
    ///
    /// Ordinary messages use [`reply::SyncKind`](crate::reply::SyncKind).
    /// Messages derived with `#[message(stream = ...)]` use
    /// [`reply::StreamKind`](crate::reply::StreamKind). Explicit-strategy
    /// messages use [`reply::RawKind`](crate::reply::RawKind) for
    /// `#[message(raw = ...)]` and
    /// [`reply::RawStreamKind`](crate::reply::RawStreamKind) for
    /// `#[message(raw_stream = ...)]`. The runtime reads this kind when
    /// selecting the [`DispatchHandler`] implementation.
    type Kind: crate::reply::ReplyKind;
}

/// Marks a [`Message`] with an explicitly selected reply type.
///
/// `#[derive(Message)]` without `#[message(reply = Type)]` is send-only and
/// does not implement this trait, so [`crate::ActorRef::call`] and
/// [`crate::ActorRef::try_call`] are unavailable for it.
pub trait HasReply: Message {}

/// Internal dispatch shape for one message type.
///
/// The runtime uses this raw shape after pairing a message with its
/// [`Message::Kind`]. Public handlers are adapted to it through blanket impls:
///
/// - [`Handler`] adapts [`SyncKind`] messages.
/// - [`StreamHandler`] adapts [`StreamKind`] stream messages.
/// - [`RawHandler`] adapts [`RawKind`](crate::reply::RawKind) messages.
/// - [`RawStreamHandler`] adapts [`RawStreamKind`](crate::reply::RawStreamKind)
///   stream messages.
///
/// Do not implement this trait directly; implement one of the public handler
/// traits above instead.
pub trait DispatchHandler<M: Message, K: crate::reply::ReplyKind = SyncKind>: HasMailbox {
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
    /// The `K` parameter selects the reply channel shape. [`SyncKind`] is the
    /// default for ordinary messages. [`StreamKind`] is used by stream messages
    /// and is selected automatically when the message implements
    /// [`StreamMessage`](crate::reply::StreamMessage). [`RawKind`] and
    /// [`RawStreamKind`](crate::reply::RawStreamKind) select explicit raw reply
    /// strategies.
    ///
    /// The precise `use<Self, M, K>` capture list excludes the lifetimes of this
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
    /// use loac::{ActorScope, DispatchHandler, IntoReply, Message};
    ///
    /// fn detach_reply<A, M>(
    ///     actor: &mut A,
    ///     message: M,
    ///     scope: &mut ActorScope<'_, A>,
    /// ) -> impl IntoReply<A, M> + use<A, M>
    /// where
    ///     A: DispatchHandler<M>,
    ///     M: Message,
    /// {
    ///     actor.handle(message, scope)
    /// }
    /// ```
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<Self, M, K>;
}

/// Handles one message with an actor-access `cx` future.
///
/// This is the primary handler trait. The runtime polls the returned future on
/// the actor's interleaved lane, so dispatch through this trait requires
/// [`HasInterleaving`]. Inside the future, use [`Cx::with`] for temporary
/// actor and scope access; pass `_` for the borrow you do not need. `with`
/// returns before any `await`; neither the actor nor scope borrow can escape
/// its call.
///
/// A unit reply needs no explicit return expression:
///
/// ```
/// use loac::{Actor, ActorScope, Cx, Handler, Message, actor};
///
/// struct Worker {
///     notifications: usize,
/// }
///
/// #[actor(mailbox, interleaved = unbounded)]
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
/// impl Handler<Notify> for Worker {
///     async fn handle(_message: Notify, mut cx: Cx<'_, Self>) {
///         cx.with(|actor, _| actor.notifications += 1);
///     }
/// }
/// ```
pub trait Handler<M: Message>: HasMailbox {
    /// Starts handling `message` and returns the reply-producing future.
    ///
    /// The future is polled on the actor's interleaved lane. `cx` provides
    /// temporary synchronous actor and scope access.
    fn handle<'a>(message: M, cx: Cx<'a, Self>) -> impl Future<Output = M::Reply> + Send + 'a;
}

#[allow(unsafe_code)]
impl<A, M> DispatchHandler<M, SyncKind> for A
where
    A: Handler<M> + HasInterleaving,
    M: Message<Kind = SyncKind>,
{
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<A, M> {
        let cx = Cx::new(self, scope.state);
        let future = Box::pin(<A as Handler<M>>::handle(message, cx))
            as Pin<Box<dyn Future<Output = M::Reply> + Send + '_>>;
        // SAFETY: the future's `'_` lifetime comes only from the `Cx` handle,
        // whose lifetime is a phantom over raw actor/scope pointers. The reply
        // is polled only on the actor task and is dropped before the actor or
        // scope state is torn down.
        let future: Pin<Box<dyn Future<Output = M::Reply> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        CxReply {
            future,
            _actor: std::marker::PhantomData,
        }
    }
}

/// Handles one message with an explicit reply strategy.
///
/// Implement this trait when a handler must choose among [`IntoReply`]
/// strategies such as [`ReplyExt::ready`](crate::ReplyExt::ready),
/// [`ReplyExt::exclusive`](crate::ReplyExt::exclusive), a bare future, or
/// [`Either`](crate::reply::Either). The message's [`Message::Kind`] must be
/// [`RawKind`](crate::reply::RawKind), selected by `#[message(raw = Type)]`.
///
/// Prefer [`Handler`] when an `async fn` with [`Cx`] is enough.
pub trait RawHandler<M: Message>: HasMailbox {
    /// Synchronously starts handling `message` and chooses its reply semantics.
    ///
    /// The returned [`IntoReply`] strategy selects owned, exclusive, or
    /// interleaved scheduling.
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<Self, M>;
}

impl<A, M> DispatchHandler<M, RawKind> for A
where
    A: RawHandler<M>,
    M: Message<Kind = RawKind>,
{
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<A, M> {
        <A as RawHandler<M>>::handle(self, message, scope)
    }
}

/// Handles a stream message with an actor-access `cx` future.
///
/// The runtime creates a bounded item channel and a final-value channel before
/// calling [`handle`](Self::handle). The returned future owns the sender side
/// and produces the final value; the caller receives the
/// [`StreamReply`](crate::reply::StreamReply) handle.
///
/// The runtime polls the returned future on the actor's interleaved lane, so
/// dispatch through this trait requires [`HasInterleaving`].
/// `out` is the runtime-created item writer wrapped in [`StreamOut`]. The
/// wrapper ties the writer to the handler future's borrow, so it cannot be
/// moved into a `'static` task; dropping it closes the caller's item stream.
pub trait StreamHandler<M>: HasMailbox
where
    M: StreamMessage,
{
    /// Starts producing stream items and returns the final reply future.
    ///
    /// `cx` provides temporary synchronous actor and scope access.
    fn handle<'a, W>(
        message: M,
        out: StreamOut<'a, W>,
        cx: Cx<'a, Self>,
    ) -> impl Future<Output = M::Final> + Send + 'a
    where
        W: Writer<M::Item> + Send + 'a;
}

#[allow(unsafe_code)]
impl<A, M> DispatchHandler<M, StreamKind> for A
where
    A: StreamHandler<M> + HasInterleaving,
    M: StreamMessage,
{
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<A, M> {
        let (item_tx, item_rx) = mpsc::channel::<M::Item>(8);
        let (final_tx, final_rx) = oneshot::channel::<M::Final>();
        let cx = Cx::new(self, scope.state);
        let out = StreamOut::new(item_tx);
        let future = Box::pin(<A as StreamHandler<M>>::handle(message, out, cx))
            as Pin<Box<dyn Future<Output = M::Final> + Send + '_>>;
        // SAFETY: the future's `'_` lifetime comes only from the `Cx` handle
        // and the `StreamOut` wrapper, both of which carry phantom lifetimes
        // over raw actor/scope pointers and the owned item writer. The reply is
        // polled only on the actor task and is dropped before the actor or
        // scope state is torn down.
        let future: Pin<Box<dyn Future<Output = M::Final> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        let strategy = CxStream {
            future,
            _actor: std::marker::PhantomData,
        };
        StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

/// Handles a stream message with an explicit final reply strategy.
///
/// Implement this trait when a stream handler must choose among
/// [`IntoStreamReply`] strategies such as a bare final future,
/// [`ReplyExt::ready`](crate::ReplyExt::ready),
/// [`ReplyExt::exclusive`](crate::ReplyExt::exclusive), or
/// [`Either`](crate::reply::Either). The message kind must be
/// [`RawStreamKind`](crate::reply::RawStreamKind), selected by
/// `#[message(raw_stream = Item, reply = Final)]`.
pub trait RawStreamHandler<M>: HasMailbox
where
    M: StreamMessage<RawStreamKind>,
{
    /// Starts producing stream items and returns the final reply strategy.
    ///
    /// This is the explicit-strategy counterpart of [`StreamHandler::handle`].
    fn handle<W>(
        &mut self,
        message: M,
        out: W,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoStreamReply<Self, M> + use<Self, M, W>
    where
        W: Writer<M::Item> + Send + 'static;
}

impl<A, M> DispatchHandler<M, RawStreamKind> for A
where
    A: RawStreamHandler<M>,
    M: StreamMessage<RawStreamKind>,
{
    fn handle(
        &mut self,
        message: M,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, M> + use<A, M> {
        let (item_tx, item_rx) = mpsc::channel::<M::Item>(8);
        let (final_tx, final_rx) = oneshot::channel::<M::Final>();
        let strategy = <A as RawStreamHandler<M>>::handle(self, message, item_tx, scope);
        StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}
