use super::*;
use crate::access::Cx;

// Runtime ownership stays private.
// Public scope views expose only phase-valid capabilities.
pub(crate) struct ScopeState<A: Actor> {
    pub(crate) actor_ref: ActorRef<A>,
    pub(crate) children: <A as SupervisionConfig>::Children,
}

impl<A: Actor> ScopeState<A> {
    // This bridge keeps sealed profile details out of scheduler code.
    pub(crate) fn children(&mut self) -> &mut impl RuntimeChildren {
        self.children.__runtime(Seal)
    }

    /// Lends the capabilities valid before child cleanup.
    pub(crate) fn actor_scope(&mut self) -> ActorScope<'_, A> {
        ActorScope { state: self }
    }

    /// Polls exits without exposing child storage to schedulers.
    pub(crate) fn poll_child_exit(&mut self, task: &mut Context<'_>) -> Poll<ChildExit> {
        self.children().poll_exit(task)
    }

    /// Lends the restricted cleanup capabilities.
    /// The exclusive borrow avoids requiring child state to be `Sync`.
    pub(crate) fn stop_scope(&mut self) -> StopScope<'_, A> {
        StopScope {
            actor_ref: &self.actor_ref,
        }
    }
}

/// Runtime capabilities available during [`Actor::on_stop`].
///
/// Child cleanup finishes before this capability is issued.
/// It cannot change the actor's child topology.
/// The runtime constructs this borrowed view after child cleanup.
/// The stop hook may keep it across `await`.
pub struct StopScope<'a, A: Actor> {
    actor_ref: &'a ActorRef<A>,
}

impl<A: Actor> StopScope<'_, A> {
    /// Returns this actor's non-owning address.
    ///
    /// Message admission is already closed during `on_stop`.
    pub const fn myself(&self) -> &ActorRef<A> {
        self.actor_ref
    }

    /// Requests shutdown while graceful cleanup is running.
    ///
    /// Stop or Drain has already committed at this stage.
    /// Kill may still upgrade either mode.
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.actor_ref.request_shutdown(shutdown)
    }
}

impl<A: Actor> fmt::Debug for StopScope<'_, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StopScope")
            .field("actor_ref", self.myself())
            .finish_non_exhaustive()
    }
}

/// Runtime capabilities available before child cleanup begins.
///
/// The parent runtime owns every child actor. Actor state may keep a returned
/// [`Child`] or [`ActorRef`], but those values do not own the child.
/// Graceful shutdown waits for every retained child actor.
/// An unconfirmed descendant remains unconfirmed in the parent's final status.
/// [`Actor::on_stop`] receives [`StopScope`] instead.
///
/// The runtime constructs this borrowed view for user actor work.
/// Handlers use it only during dispatch.
/// Actor futures receive a fresh view for each poll.
/// Initialization and lifecycle hooks may retain it across `await`.
pub struct ActorScope<'a, A: Actor> {
    pub(crate) state: &'a mut ScopeState<A>,
}

impl<A: Actor> ActorScope<'_, A> {
    /// Returns this actor's non-owning address.
    ///
    /// Owned replies consume no interleaved capacity.
    /// Their self-calls still require scheduler dispatch capacity.
    /// Actors without interleaving have no interleaved capacity gate.
    /// An interleaved reply needs another slot for its self-call.
    /// An exclusive reply blocks its queued self-call until it ends.
    ///
    /// A serial lifecycle hook also blocks dispatch. While admission is still
    /// open, as in `init` or a running actor's `on_child_exit`, awaiting an
    /// accepted self-call likewise waits until Kill or executor teardown. Once
    /// shutdown closes admission, [`ActorRef::call`] returns
    /// [`CallError::Closed`](crate::CallError::Closed) and
    /// [`ActorRef::try_call`] reports
    /// [`TryCallErrorKind::Closed`](crate::TryCallErrorKind::Closed) instead.
    pub const fn myself(&self) -> &ActorRef<A> {
        &self.state.actor_ref
    }

    /// Requests shutdown of this actor and, eventually, its subtree.
    ///
    /// This has the same first-wins and Kill-upgrade behavior as
    /// [`ActorOwner::request_shutdown`]. It commits synchronously, but Kill is
    /// cooperative: the current handler or poll returns before the runtime drops
    /// remaining actor work and propagates shutdown to children.
    ///
    /// Stop and Drain retain already-dispatched replies. Kill and reply
    /// completion instead commit through the same lifecycle gate, so whichever
    /// commits first determines the caller's result.
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.state.actor_ref.request_shutdown(shutdown)
    }

    /// Builds an interleaved plain-Future reply that may access actor and
    /// scope through the returned [`Cx`] handle.
    ///
    /// Use this inside [`RawHandler`](crate::RawHandler) when the reply should
    /// use cx-style access and interleaved scheduling. Interleaved scheduling
    /// requires [`HasInterleaving`](crate::HasInterleaving); use
    /// [`cx_exclusive`](Self::cx_exclusive) for the exclusive counterpart.
    ///
    /// The closure receives a [`Cx`] handle and must return a boxed future
    /// tied to the handle lifetime. Inside that future, call
    /// [`Cx::with`](crate::Cx::with) for temporary actor and scope access.
    /// The runtime erases the future lifetime internally; safe code cannot
    /// store the handle in a `'static` location.
    #[allow(unsafe_code)]
    pub fn cx_reply<R, F>(&mut self, actor: &mut A, f: F) -> crate::reply::CxReply<A, R>
    where
        F: for<'a> FnOnce(
            Cx<'a, A>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'a>>,
    {
        let cx = Cx::new(actor, self.state);
        let future = f(cx);
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        crate::reply::CxReply {
            future,
            _actor: std::marker::PhantomData,
        }
    }

    /// Streaming interleaved counterpart of [`ActorScope::cx_reply`].
    ///
    /// Use this inside [`RawStreamHandler`](crate::RawStreamHandler) when the
    /// stream-final reply should use cx-style access and interleaved
    /// scheduling. Interleaved scheduling requires
    /// [`HasInterleaving`](crate::HasInterleaving); see
    /// [`cx_stream_exclusive`](Self::cx_stream_exclusive) for the exclusive
    /// counterpart.
    #[allow(unsafe_code)]
    pub fn cx_stream<R, F>(&mut self, actor: &mut A, f: F) -> crate::reply::CxStream<A, R>
    where
        F: for<'a> FnOnce(
            Cx<'a, A>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'a>>,
    {
        let cx = Cx::new(actor, self.state);
        let future = f(cx);
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        crate::reply::CxStream {
            future,
            _actor: std::marker::PhantomData,
        }
    }

    /// Builds an exclusive plain-Future reply that may access actor and scope
    /// through the returned [`Cx`] handle.
    ///
    /// Use this inside [`RawHandler`](crate::RawHandler) when the reply should
    /// use cx-style access and exclusive scheduling. Exclusive scheduling
    /// pauses mailbox dispatch and other actor-aware work until the future
    /// finishes; owned tasks may continue. It does not require
    /// [`HasInterleaving`](crate::HasInterleaving).
    ///
    /// The closure receives a [`Cx`] handle and must return a boxed future
    /// tied to the handle lifetime. Inside that future, call
    /// [`Cx::with`](crate::Cx::with) for temporary actor and scope access.
    /// The runtime erases the future lifetime internally; safe code cannot
    /// store the handle in a `'static` location.
    #[allow(unsafe_code)]
    pub fn cx_exclusive<R, F>(&mut self, actor: &mut A, f: F) -> crate::reply::CxExclusive<A, R>
    where
        F: for<'a> FnOnce(
            Cx<'a, A>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'a>>,
    {
        let cx = Cx::new(actor, self.state);
        let future = f(cx);
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        crate::reply::CxExclusive {
            future,
            _actor: std::marker::PhantomData,
        }
    }

    /// Streaming exclusive counterpart of [`ActorScope::cx_exclusive`].
    ///
    /// Use this inside [`RawStreamHandler`](crate::RawStreamHandler) when the
    /// stream-final reply should use cx-style access and exclusive scheduling.
    /// Exclusive scheduling pauses mailbox dispatch and other actor-aware work
    /// until the final future finishes; owned tasks may continue. It does not
    /// require [`HasInterleaving`](crate::HasInterleaving). The returned future
    /// may also capture the item writer passed to the raw stream handler.
    #[allow(unsafe_code)]
    pub fn cx_stream_exclusive<R, F>(
        &mut self,
        actor: &mut A,
        f: F,
    ) -> crate::reply::CxStreamExclusive<A, R>
    where
        F: for<'a> FnOnce(
            Cx<'a, A>,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'a>>,
    {
        let cx = Cx::new(actor, self.state);
        let future = f(cx);
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = R> + Send + 'static>> =
            unsafe { std::mem::transmute(future) };
        crate::reply::CxStreamExclusive {
            future,
            _actor: std::marker::PhantomData,
        }
    }
}

impl<A: HasChildren> ActorScope<'_, A> {
    /// Spawns and owns one direct child actor.
    ///
    /// Child registration commits synchronously.
    /// Child initialization then runs asynchronously.
    /// Its mailbox accepts before initialization finishes.
    /// Registration completes before the child task can start.
    ///
    /// Finite profiles return [`Full`](crate::supervision::Full) at capacity.
    /// The error retains `args` without opening child configuration.
    /// Recover `args` through [`Full::into_inner`](crate::supervision::Full::into_inner).
    /// Unbounded profiles use [`Infallible`](std::convert::Infallible).
    /// Capacity counts retained direct-child registrations.
    /// A dequeued exit still retains its registration.
    /// Reaping releases capacity before [`Actor::on_child_exit`].
    ///
    /// The returned [`Child`] does not own lifecycle.
    /// Retained graceful work keeps the actor scope capability valid.
    /// A concurrent Kill cannot interrupt the current poll.
    pub fn spawn_child<C: Actor>(
        &mut self,
        args: C::SpawnArgs,
    ) -> Result<Child<C>, <A::Children as ChildSpawner>::Error<C::SpawnArgs>> {
        let args = self.state.children.__runtime_spawner(Seal).admit(args)?;
        let prepared = PreparedActor::new(args, SpawnOptions::<C>::default());
        let registered = self
            .state
            .children
            .__runtime_spawner(Seal)
            .register(prepared);
        Ok(start_child(registered))
    }

    /// Spawns and owns one direct child actor with explicit options.
    ///
    /// Registration and initialization follow [`spawn_child`](Self::spawn_child).
    ///
    /// A finite rejection retains `(args, options)`.
    /// [`Full::into_inner`](crate::supervision::Full::into_inner) returns that tuple.
    /// Child configuration remains unopened after rejection.
    ///
    /// The returned [`Child`] does not own lifecycle.
    /// Retained graceful work keeps the actor scope capability valid.
    /// A concurrent Kill cannot interrupt the current poll.
    #[allow(
        clippy::type_complexity,
        reason = "the error preserves both rejected spawn inputs"
    )]
    pub fn spawn_child_with<C: Actor>(
        &mut self,
        args: C::SpawnArgs,
        options: SpawnOptions<C>,
    ) -> Result<Child<C>, <A::Children as ChildSpawner>::Error<(C::SpawnArgs, SpawnOptions<C>)>>
    {
        let (args, options) = self
            .state
            .children
            .__runtime_spawner(Seal)
            .admit((args, options))?;
        let prepared = PreparedActor::new(args, options);
        let registered = self
            .state
            .children
            .__runtime_spawner(Seal)
            .register(prepared);
        Ok(start_child(registered))
    }
}

impl<A: Actor> fmt::Debug for ActorScope<'_, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorScope")
            .field("actor_ref", self.myself())
            .finish_non_exhaustive()
    }
}
