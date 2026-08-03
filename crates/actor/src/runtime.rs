use std::{
    fmt,
    future::Future,
    num::NonZeroUsize,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::FutureExt;
use pin_project_lite::pin_project;
use slotmap::{DefaultKey, SlotMap};
use tokio::sync::mpsc;

use crate::{
    Actor, ActorRef, Child, ChildExit, ChildId, ErasedFuture, ExitReason, ExitStatus, Shutdown,
    ShutdownStatus, SubtreeStatus,
    mailbox::{ActorInbox, ActorInner, Control, HookEntryPermit, Mode},
    owned::OwnedTasks,
    scheduler::{InterleavedPoll, ReplyScheduler},
};

/// Configuration applied when one actor is spawned.
///
/// Mailbox capacity bounds accepted work waiting for dispatch.
/// The in-flight limit bounds active interleaved replies.
/// Both limits default to 32.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnOptions {
    mailbox_capacity: NonZeroUsize,
    max_in_flight: NonZeroUsize,
}

impl SpawnOptions {
    /// Sets the maximum number of queued, not-yet-dequeued messages.
    pub const fn with_mailbox_capacity(mut self, mailbox_capacity: NonZeroUsize) -> Self {
        self.mailbox_capacity = mailbox_capacity;
        self
    }

    /// Sets the maximum number of active interleaved replies.
    ///
    /// At the limit, new mailbox dispatch pauses.
    /// This also delays ready or owned handler dispatch.
    /// The runtime learns the reply mode only after dispatch.
    /// Owned tasks are unbounded and consume no slot.
    /// Exclusive work runs alone among actor-aware replies.
    /// No slot is reserved for self-calls.
    pub const fn with_max_in_flight(mut self, max_in_flight: NonZeroUsize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    /// Returns the maximum number of queued, not-yet-dequeued messages.
    pub const fn mailbox_capacity(self) -> NonZeroUsize {
        self.mailbox_capacity
    }

    /// Returns the maximum number of active interleaved replies.
    ///
    /// See [`with_max_in_flight`](Self::with_max_in_flight) for the dispatch and
    /// self-call behavior governed by this limit.
    pub const fn max_in_flight(self) -> NonZeroUsize {
        self.max_in_flight
    }
}

impl Default for SpawnOptions {
    fn default() -> Self {
        let default = NonZeroUsize::new(32).expect("the default limits are non-zero");
        Self {
            mailbox_capacity: default,
            max_in_flight: default,
        }
    }
}

/// Spawns a root actor with [`SpawnOptions::default`].
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
    spawn_with::<A>(args, SpawnOptions::default())
}

/// Spawns a root actor with explicit options.
///
/// Initialization and admission follow [`spawn`].
/// The returned [`ActorOwner`] owns the actor lifecycle.
/// This function requires an active Tokio runtime.
#[must_use = "dropping the returned owner requests Kill"]
pub fn spawn_with<A: Actor>(args: A::SpawnArgs, options: SpawnOptions) -> ActorOwner<A> {
    ActorOwner(PreparedActor::new(args, options).start(None))
}

/// The unique lifecycle owner of a root actor.
///
/// This type is deliberately not cloneable. Dropping it requests a best-effort
/// Kill but cannot synchronously wait from `Drop`; use [`shutdown`](
/// Self::shutdown) or [`wait`](Self::wait) when confirmed normal-path subtree
/// termination matters. [`ExitStatus`] separates the actor's reason from its
/// subtree guarantee.
#[must_use = "dropping an actor owner requests Kill"]
pub struct ActorOwner<A: Actor>(ActorRef<A>);

impl<A: Actor> ActorOwner<A> {
    /// Returns a cloneable, non-owning address.
    pub fn actor_ref(&self) -> ActorRef<A> {
        self.0.clone()
    }

    /// Requests Stop, Drain, or Kill without waiting for completion.
    ///
    /// The request atomically closes admission if it establishes a mode. Stop
    /// and Drain are first-wins peers, while Kill may upgrade either one. See
    /// [`Shutdown`] for retained work and cleanup behavior, and
    /// [`ShutdownStatus`] for the meaning of the immediate result.
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.0.request_shutdown(shutdown)
    }

    /// Returns the terminal status if the actor has already exited.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.0.exit_status()
    }

    /// Waits for the actor to publish its terminal event.
    ///
    /// This method does not initiate shutdown. Keeping `&mut self` allows a
    /// caller to apply an external deadline and upgrade to Kill afterward.
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
        self.request_shutdown(shutdown);
        self.wait().await
    }
}

impl<A: Actor> Drop for ActorOwner<A> {
    fn drop(&mut self) {
        self.0.request_shutdown(Shutdown::Kill);
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

// Runtime ownership stays private.
// Public scope views expose only phase-valid capabilities.
struct ScopeState<A: Actor> {
    actor_ref: ActorRef<A>,
    children: ChildSet,
    supervisor_tx: mpsc::UnboundedSender<ChildExit>,
}

impl<A: Actor> ScopeState<A> {
    /// Lends the capabilities valid before child cleanup.
    fn actor_scope(&mut self) -> ActorScope<'_, A> {
        ActorScope { state: self }
    }

    /// Lends the restricted cleanup capabilities.
    fn stop_scope(&self) -> StopScope<'_, A> {
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
    state: &'a mut ScopeState<A>,
}

impl<A: Actor> ActorScope<'_, A> {
    /// Returns this actor's non-owning address.
    ///
    /// An owned reply consumes no interleaved slot.
    /// Its self-call can progress while a slot remains available.
    /// An interleaved reply needs one additional slot for its self-call.
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

    /// Spawns and owns one direct child actor.
    ///
    /// Child registration commits synchronously.
    /// Child initialization then runs asynchronously.
    /// Its mailbox accepts before initialization finishes.
    ///
    /// The returned [`Child`] does not own lifecycle.
    /// Retained graceful work keeps this capability.
    /// A concurrent Kill cannot interrupt the current poll.
    pub fn spawn_child<C: Actor>(&mut self, args: C::SpawnArgs) -> Child<C> {
        self.spawn_child_with::<C>(args, SpawnOptions::default())
    }

    /// Spawns and owns one direct child actor with explicit options.
    ///
    /// Registration and initialization follow [`spawn_child`](Self::spawn_child).
    ///
    /// The returned [`Child`] does not own lifecycle.
    /// Retained graceful work keeps this capability.
    /// A concurrent Kill cannot interrupt the current poll.
    pub fn spawn_child_with<C: Actor>(
        &mut self,
        args: C::SpawnArgs,
        options: SpawnOptions,
    ) -> Child<C> {
        self.state
            .children
            .spawn::<C>(args, options, self.state.supervisor_tx.clone())
    }
}

impl<A: Actor> fmt::Debug for ActorScope<'_, A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorScope")
            .field("actor_ref", self.myself())
            .field("children", &self.state.children.len())
            .finish_non_exhaustive()
    }
}

struct ParentLink {
    id: ChildId,
    events: mpsc::UnboundedSender<ChildExit>,
}

/// Builds actor communication state without scheduling actor code.
///
/// A child must receive its parent-issued key before its task can exit.
/// Preparation keeps that ordering explicit without placeholder state.
struct PreparedActor<A: Actor> {
    actor_ref: ActorRef<A>,
    future: ErasedFuture<'static, ExitStatus>,
}

/// Cold ownership operations for heterogeneous child storage.
///
/// The trait object points at the same allocation as its typed ActorRef.
/// It exposes no message capability to ChildSet.
trait ErasedActor: Send + Sync {
    fn control(&self) -> &Control;
}

impl<A: Actor> ErasedActor for ActorInner<A> {
    fn control(&self) -> &Control {
        &self.control
    }
}

struct ErasedActorOwner(Arc<dyn ErasedActor>);

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

struct ChildSet {
    actors: SlotMap<DefaultKey, ErasedActorOwner>,
    // Removed children cannot erase a lost subtree guarantee.
    subtree: SubtreeStatus,
}

impl Default for ChildSet {
    fn default() -> Self {
        Self {
            actors: SlotMap::new(),
            subtree: SubtreeStatus::Terminated,
        }
    }
}

impl ChildSet {
    fn len(&self) -> usize {
        self.actors.len()
    }

    /// Issues the storage key before constructing the child's parent link.
    /// The parent cannot consume an exit until this insertion returns.
    fn spawn<A: Actor>(
        &mut self,
        args: A::SpawnArgs,
        options: SpawnOptions,
        events: mpsc::UnboundedSender<ChildExit>,
    ) -> Child<A> {
        let prepared = PreparedActor::new(args, options);
        let child_ref = prepared.actor_ref.clone();
        let key = self.actors.insert_with_key(move |key| {
            let parent = ParentLink {
                id: ChildId::from_key(key),
                events,
            };
            let actor_ref = prepared.start(Some(parent));
            ErasedActorOwner::new(&actor_ref)
        });
        Child::new(ChildId::from_key(key), child_ref)
    }

    #[cfg(test)]
    /// Installs an already-started fixture without a parent notification link.
    fn insert(&mut self, actor: ErasedActorOwner) -> ChildId {
        ChildId::from_key(self.actors.insert(actor))
    }

    fn remove(&mut self, event: &ChildExit) -> bool {
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
        // Persist each unconfirmed result before another cancellation point.
        let Self { actors, subtree } = self;
        for actor in actors.values() {
            if actor.wait().await.subtree() == SubtreeStatus::Unconfirmed {
                *subtree = SubtreeStatus::Unconfirmed;
            }
        }
        actors.clear();
    }

    /// Combines this actor's reason with the retained subtree guarantee.
    fn terminal_status(&self, reason: ExitReason) -> ExitStatus {
        ExitStatus::new(reason, self.subtree)
    }
}

impl<A: Actor> PreparedActor<A> {
    fn new(args: A::SpawnArgs, options: SpawnOptions) -> Self {
        let (inner, inbox) = ActorInner::channel(options.mailbox_capacity().get());
        let actor_ref = ActorRef::new(inner);

        // Each actor receives terminal events from its direct children.
        // The nonblocking channel prevents child teardown from awaiting parent work.
        let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
        let state = ScopeState {
            actor_ref: actor_ref.clone(),
            children: ChildSet::default(),
            supervisor_tx,
        };
        let future = Box::pin(run_actor(
            args,
            state,
            inbox,
            supervisor_rx,
            options.max_in_flight(),
        ));

        Self { actor_ref, future }
    }

    /// Starts the detached task after its complete parent link exists.
    fn start(self, parent: Option<ParentLink>) -> ActorRef<A> {
        let Self { actor_ref, future } = self;
        let exit = ExitGuard::new(Arc::clone(&actor_ref.0), parent);
        drop(tokio::spawn(ActorTask::new(future, exit)));
        actor_ref
    }
}

struct ExitGuard<A: Actor> {
    actor: Arc<ActorInner<A>>,
    parent: Option<ParentLink>,
    finished: bool,
}

impl<A: Actor> ExitGuard<A> {
    fn new(actor: Arc<ActorInner<A>>, parent: Option<ParentLink>) -> Self {
        Self {
            actor,
            parent,
            finished: false,
        }
    }

    fn prepare_abort(&self) {
        self.actor.control.begin_abort();
    }

    fn complete(mut self, status: ExitStatus) -> ExitStatus {
        self.publish(status)
    }

    fn publish(&mut self, proposed: ExitStatus) -> ExitStatus {
        if self.finished {
            return self
                .actor
                .control
                .exit_status()
                .expect("a finished guard published an exit status");
        }
        self.finished = true;
        let status = self.actor.control.finish(proposed);
        if let Some(parent) = &self.parent {
            let _ = parent.events.send(ChildExit::new(parent.id, status));
        }
        status
    }
}

impl<A: Actor> Drop for ExitGuard<A> {
    fn drop(&mut self) {
        if !self.finished {
            // An unfinished guard means ActorTask did not complete publication.
            // Mark abort before publishing its conservative status.
            self.prepare_abort();
            let status = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            let _ = self.publish(status);
        }
    }
}

struct ActorTask<A: Actor> {
    future: Option<ErasedFuture<'static, ExitStatus>>,
    exit: Option<ExitGuard<A>>,
}

impl<A: Actor> ActorTask<A> {
    fn new(future: ErasedFuture<'static, ExitStatus>, exit: ExitGuard<A>) -> Self {
        Self {
            future: Some(future),
            exit: Some(exit),
        }
    }
}

impl<A: Actor> Future for ActorTask<A> {
    type Output = ExitStatus;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = this
            .future
            .as_mut()
            .expect("completed actor tasks are not polled again")
            .as_mut()
            .poll(context);

        let Poll::Ready(status) = result else {
            return Poll::Pending;
        };

        // Frame destruction can fail before terminal publication.
        // The lifecycle gate preserves any Kill that already committed.
        let exit = this.exit.take().expect("an actor task owns one exit guard");
        let future = this
            .future
            .take()
            .expect("a ready actor task owns one final frame");
        exit.actor.control.drop_user_value(future);
        let status = exit.complete(status);
        Poll::Ready(status)
    }
}

impl<A: Actor> Drop for ActorTask<A> {
    fn drop(&mut self) {
        let Some(exit) = self.exit.take() else {
            return;
        };
        // Aborting records a local abort before frame destruction.
        // The guard publishes only after contained cleanup returns.
        exit.prepare_abort();
        if let Some(future) = self.future.take() {
            exit.actor.control.drop_user_value(future);
        }
        drop(exit);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Work<T = ()> {
    Complete(T),
    Killed,
    Panicked,
    // The ready output still belongs to the lifecycle owner.
    // It may require child-first teardown before being dropped.
    DropPanicked(T),
}

pin_project! {
    // `project_replace` leaves `Done` if future Drop panics.
    // `PinnedDrop` therefore cannot drop the future twice.
    // Separate state keeps replacement independent from guard destruction.
    #[project = ActorWorkStateProj]
    #[project_replace = ActorWorkStateProjReplace]
    enum ActorWorkState<F> {
        Running {
            #[pin]
            future: F,
        },
        Done,
    }
}

pin_project! {
    /// Contains polling and destruction for one pinned lifecycle future.
    struct ActorWorkGuard<'a, F> {
        #[pin]
        state: ActorWorkState<F>,
        control: &'a Control,
    }

    impl<F> PinnedDrop for ActorWorkGuard<'_, F> {
        fn drop(this: Pin<&mut Self>) {
            let _ = this.drop_future_panicked();
        }
    }
}

impl<'a, F> ActorWorkGuard<'a, F> {
    /// Retires the pinned future and reports a contained Drop panic.
    fn drop_future_panicked(mut self: Pin<&mut Self>) -> bool {
        let this = self.as_mut().project();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = this.state.project_replace(ActorWorkState::Done);
        }));
        if let Err(payload) = result {
            this.control.contain_panic(payload);
            true
        } else {
            false
        }
    }
}

impl<F, T> Future for ActorWorkGuard<'_, F>
where
    F: Future<Output = T>,
{
    type Output = Work<T>;

    fn poll(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        let result = {
            let this = self.as_mut().project();
            let ActorWorkStateProj::Running { future } = this.state.project() else {
                unreachable!("completed actor work cannot be polled");
            };
            panic::catch_unwind(AssertUnwindSafe(|| future.poll(task)))
        };

        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => {
                if self.as_mut().drop_future_panicked() {
                    Poll::Ready(Work::DropPanicked(output))
                } else {
                    Poll::Ready(Work::Complete(output))
                }
            }
            Err(payload) => {
                self.as_mut().project().control.contain_panic(payload);
                let _ = self.as_mut().drop_future_panicked();
                Poll::Ready(Work::Panicked)
            }
        }
    }
}

async fn await_actor_work<F>(future: F, control: &Control) -> Work<F::Output>
where
    F: Future + Send,
{
    let guarded = ActorWorkGuard {
        state: ActorWorkState::Running { future },
        control,
    };
    tokio::pin!(guarded);

    loop {
        if matches!(
            control.mode(),
            Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_)
        ) {
            return Work::Killed;
        }

        tokio::select! {
            biased;
            () = control.actor_notified() => {}
            result = &mut guarded => return result,
        }
    }
}

async fn run_actor<A: Actor>(
    args: A::SpawnArgs,
    mut state: ScopeState<A>,
    mut inbox: ActorInbox<A>,
    mut supervisor_rx: mpsc::UnboundedReceiver<ChildExit>,
    max_in_flight: NonZeroUsize,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    let owned = OwnedTasks::new(Arc::clone(&inner));
    let mut scheduler = ReplyScheduler::new(max_in_flight);
    let mut turn_cursor = TurnCursor::default();

    let initialized = if let Some(_permit) = control.begin_initialization() {
        let mut scope = state.actor_scope();
        match panic::catch_unwind(AssertUnwindSafe(|| A::init(args, &mut scope))) {
            Ok(init) => await_actor_work(init, control).await,
            Err(payload) => {
                control.contain_panic(payload);
                Work::Panicked
            }
        }
    } else {
        control.drop_user_value(args);
        Work::Killed
    };

    let mut actor = match initialized {
        Work::Complete(actor) => actor,
        Work::Killed => return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await,
        Work::Panicked => return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await,
        Work::DropPanicked(actor) => {
            // The init frame failed after producing actor state.
            // Descendant cancellation must precede arbitrary actor Drop code.
            state.children.request_all(Shutdown::Kill);
            control.drop_user_value(actor);
            return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
        }
    };

    loop {
        match control.mode() {
            Mode::Running => {}
            Mode::Draining => {
                return drain_actor(
                    &mut actor,
                    &mut state,
                    &mut inbox,
                    &mut supervisor_rx,
                    &inner,
                    &owned,
                    &mut scheduler,
                    &mut turn_cursor,
                )
                .await;
            }
            Mode::Stopping => {
                return stop_actor(
                    &mut actor,
                    &mut state,
                    &mut inbox,
                    control,
                    &owned,
                    &mut scheduler,
                )
                .await;
            }
            Mode::Killing => {
                return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Exited(status) => return status,
            Mode::Aborting => {
                return ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            }
        }

        let turn = AssertUnwindSafe(actor_turn(
            &mut actor,
            &mut state,
            &mut inbox,
            &mut supervisor_rx,
            &inner,
            &owned,
            &mut scheduler,
            true,
            Mode::Running,
            &mut turn_cursor,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(payload) => {
                control.contain_panic(payload);
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
        };

        match turn {
            Turn::LifecycleHint | Turn::ReplyProgress => {}
            Turn::RepliesFinished => {
                unreachable!("a running actor cannot finish reply scheduling")
            }
            Turn::Child(event) => {
                match handle_child_exit(&mut actor, &mut state, event, control).await {
                    Work::Complete(()) => {}
                    Work::Killed => {
                        return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                    Work::Panicked | Work::DropPanicked(()) => {
                        control.begin_failure();
                        return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                }
            }
            Turn::MailboxProgress => {}
            Turn::InboxClosed => {
                control.begin_failure();
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
        }
    }
}

enum Turn {
    // Notification or mismatch; the caller re-reads Control.
    LifecycleHint,
    ReplyProgress,
    // Drain handles child exits while awaiting this barrier.
    RepliesFinished,
    Child(ChildExit),
    MailboxProgress,
    InboxClosed,
}

// A running mailbox turn has a fixed dispatch budget.
// This bounds delay for interleaved and child work.
const MAILBOX_DISPATCH_BUDGET: usize = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OrdinaryLane {
    #[default]
    Mailbox,
    Interleaved,
    ChildExit,
}

impl OrdinaryLane {
    /// Defines the fixed cycle shared by ordinary actor work.
    const fn next(self) -> Self {
        match self {
            Self::Mailbox => Self::Interleaved,
            Self::Interleaved => Self::ChildExit,
            Self::ChildExit => Self::Mailbox,
        }
    }
}

#[derive(Default)]
struct TurnCursor {
    next_ordinary: OrdinaryLane,
}

#[expect(
    clippy::too_many_arguments,
    reason = "one turn borrows each independent actor-task resource"
)]
// Mailbox, interleaved, and child work share the cursor.
// Owned tasks do not share this rotation.
// Exclusive work pauses those three sources.
// Lifecycle changes are checked before the rotation.
async fn actor_turn<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    inner: &Arc<ActorInner<A>>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
    receive_messages: bool,
    expected_mode: Mode,
    cursor: &mut TurnCursor,
) -> Turn {
    let control = &inner.control;
    let wait_for_owned = !receive_messages && scheduler.is_empty();
    let mailbox_dispatch_budget = if expected_mode == Mode::Running {
        MAILBOX_DISPATCH_BUDGET
    } else {
        1
    };
    let fair_turn = std::future::poll_fn(|task| {
        if control.mode() != expected_mode {
            return Poll::Ready(Turn::LifecycleHint);
        }

        if scheduler.has_exclusive() {
            let mut scope = state.actor_scope();
            let ready = scheduler
                .poll_exclusive(actor, &mut scope, control, expected_mode, task)
                .is_ready();
            if control.mode() != expected_mode {
                return Poll::Ready(Turn::LifecycleHint);
            }
            return if ready {
                Poll::Ready(Turn::ReplyProgress)
            } else {
                Poll::Pending
            };
        }

        let start = cursor.next_ordinary;
        let mut lane = start;
        loop {
            if control.mode() != expected_mode {
                return Poll::Ready(Turn::LifecycleHint);
            }
            let selected = match lane {
                OrdinaryLane::Mailbox if receive_messages && scheduler.has_dispatch_capacity() => {
                    let mut scope = state.actor_scope();
                    let mut dispatched = 0;
                    loop {
                        match inbox.poll_recv(task) {
                            Poll::Ready(Some(envelope)) => {
                                envelope.dispatch(actor, &mut scope, owned, scheduler, inner);
                                dispatched += 1;
                            }
                            Poll::Ready(None) => break Some(Turn::InboxClosed),
                            Poll::Pending
                                if dispatched > 0
                                    && start == OrdinaryLane::Interleaved
                                    && scheduler.has_interleaved() =>
                            {
                                // Dispatch can activate a lane already visited this turn.
                                // Return progress so the new future gets its first poll.
                                break Some(Turn::MailboxProgress);
                            }
                            Poll::Pending => break None,
                        }

                        if control.mode() != expected_mode {
                            cursor.next_ordinary = lane.next();
                            return Poll::Ready(Turn::LifecycleHint);
                        }
                        if dispatched == mailbox_dispatch_budget
                            || !scheduler.has_dispatch_capacity()
                        {
                            break Some(Turn::MailboxProgress);
                        }
                    }
                }
                OrdinaryLane::Interleaved if scheduler.has_interleaved() => {
                    let mut scope = state.actor_scope();
                    match scheduler.poll_interleaved(
                        actor,
                        &mut scope,
                        control,
                        expected_mode,
                        task,
                    ) {
                        InterleavedPoll::Pending => None,
                        InterleavedPoll::Progress => Some(Turn::ReplyProgress),
                        InterleavedPoll::BudgetExhausted => {
                            // Continue from the next lane after Tokio repolls us.
                            // The scheduler already preserved and woke its sweep.
                            cursor.next_ordinary = lane.next();
                            return Poll::Pending;
                        }
                    }
                }
                OrdinaryLane::ChildExit => match supervisor_rx.poll_recv(task) {
                    Poll::Ready(Some(event)) => Some(Turn::Child(event)),
                    Poll::Ready(None) => {
                        // The runtime keeps this receiver open while scope lives.
                        // Parent runtime state retains the paired sender.
                        unreachable!("child-exit receiver closed while parent runtime was alive")
                    }
                    Poll::Pending => None,
                },
                _ => None,
            };

            if let Some(turn) = selected {
                cursor.next_ordinary = lane.next();
                return Poll::Ready(turn);
            }

            lane = lane.next();
            if lane == start {
                break;
            }
        }

        cursor.next_ordinary = start.next();
        Poll::Pending
    });

    // Lifecycle always gets first poll rights, especially Kill.
    tokio::select! {
        biased;
        () = control.actor_notified() => Turn::LifecycleHint,
        turn = fair_turn => turn,
        () = owned.wait(), if wait_for_owned => Turn::RepliesFinished,
    }
}

async fn handle_child_exit<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    event: ChildExit,
    control: &Control,
) -> Work {
    if !state.children.remove(&event) {
        return Work::Complete(());
    }

    let Some(permit) = state.actor_ref.0.control.begin_child_hook() else {
        return Work::Complete(());
    };

    run_child_exit_hook(actor, state, event, control, permit).await
}

/// Makes the private gate proof mandatory at the only user hook call site.
async fn run_child_exit_hook<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    event: ChildExit,
    control: &Control,
    _permit: HookEntryPermit,
) -> Work {
    await_actor_work(
        async {
            let mut scope = state.actor_scope();
            actor.on_child_exit(event, &mut scope).await;
        },
        control,
    )
    .await
}

async fn stop_actor<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    control: &Control,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    match close_and_discard(inbox, &state.actor_ref.0.control, Mode::Stopping).await {
        DiscardOutcome::Complete => {}
        DiscardOutcome::ModeChanged => match state.actor_ref.0.control.mode() {
            Mode::Killing => return kill_actor(state, inbox, owned, scheduler).await,
            Mode::Failing => return fail_actor(state, inbox, owned, scheduler).await,
            // Lifecycle cannot return to a graceful mode. Aborting and Exited
            // belong to ActorTask's outer drop/publication path, which cannot
            // repoll this inner future after committing either state.
            mode => unreachable!("stop discard observed impossible mode: {mode:?}"),
        },
    }
    owned.close();
    match finish_replies(actor, state, control, owned, scheduler).await {
        Work::Complete(()) => {}
        Work::Killed => return kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => {
            return fail_actor(state, inbox, owned, scheduler).await;
        }
    }

    match graceful_finish(actor, state, control, Shutdown::Stop, ExitReason::Stopped).await {
        Work::Complete(()) => state.children.terminal_status(ExitReason::Stopped),
        Work::Killed => kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => fail_actor(state, inbox, owned, scheduler).await,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "drain borrows each independent actor-task resource"
)]
async fn drain_actor<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    inner: &Arc<ActorInner<A>>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
    turn_cursor: &mut TurnCursor,
) -> ExitStatus {
    let control = &inner.control;
    // Admission physically enqueues under the lifecycle transaction, so this
    // queue is stable once Drain commits. Capacity permits that never reached
    // admission are not accepted work and must not extend graceful shutdown.
    inbox.close();
    let mut inbox_drained = inbox.is_empty();
    if inbox_drained {
        owned.close();
    }
    loop {
        match control.mode() {
            Mode::Killing => {
                return kill_actor(state, inbox, owned, scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(state, inbox, owned, scheduler).await;
            }
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Exited(status) => return status,
            Mode::Aborting => {
                return ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            }
        }

        if !inbox_drained && inbox.is_empty() {
            inbox_drained = true;
            owned.close();
        }

        let turn = AssertUnwindSafe(actor_turn(
            actor,
            state,
            inbox,
            supervisor_rx,
            inner,
            owned,
            scheduler,
            !inbox_drained,
            Mode::Draining,
            turn_cursor,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(payload) => {
                control.contain_panic(payload);
                return fail_actor(state, inbox, owned, scheduler).await;
            }
        };

        match turn {
            Turn::LifecycleHint | Turn::ReplyProgress => {}
            Turn::RepliesFinished => break,
            Turn::Child(event) => match handle_child_exit(actor, state, event, control).await {
                Work::Complete(()) => {}
                Work::Killed => {
                    return kill_actor(state, inbox, owned, scheduler).await;
                }
                Work::Panicked | Work::DropPanicked(()) => {
                    control.begin_failure();
                    return fail_actor(state, inbox, owned, scheduler).await;
                }
            },
            Turn::MailboxProgress => {}
            Turn::InboxClosed => {
                inbox_drained = true;
                owned.close();
            }
        }
    }

    match graceful_finish(actor, state, control, Shutdown::Drain, ExitReason::Drained).await {
        Work::Complete(()) => state.children.terminal_status(ExitReason::Drained),
        Work::Killed => kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => fail_actor(state, inbox, owned, scheduler).await,
    }
}

async fn finish_replies<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    control: &Control,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
) -> Work {
    while !scheduler.is_empty() {
        match control.mode() {
            Mode::Killing => return Work::Killed,
            Mode::Failing => return Work::Panicked,
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Aborting | Mode::Exited(_) => return Work::Killed,
        }

        let result = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                () = control.actor_notified() => {}
                () = std::future::poll_fn(|task| {
                    let mut scope = state.actor_scope();
                    scheduler.poll_active(actor, &mut scope, control, Mode::Stopping, task)
                }) => {}
            }
        })
        .catch_unwind()
        .await;

        if let Err(payload) = result {
            control.contain_panic(payload);
            return Work::Panicked;
        }
    }

    loop {
        match control.mode() {
            Mode::Killing => return Work::Killed,
            Mode::Failing => return Work::Panicked,
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Aborting | Mode::Exited(_) => return Work::Killed,
        }

        let result = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                () = control.actor_notified() => false,
                () = owned.wait() => true,
            }
        })
        .catch_unwind()
        .await;

        match result {
            Ok(true) => return Work::Complete(()),
            Ok(false) => {}
            Err(payload) => {
                control.contain_panic(payload);
                return Work::Panicked;
            }
        }
    }
}

/// Graceful shutdown is post-order: a parent finishes the work retained by the
/// selected mode, then waits for children, and only then runs its cleanup hook.
async fn graceful_finish<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    control: &Control,
    shutdown: Shutdown,
    reason: ExitReason,
) -> Work {
    // Keep child submission inside the biased lifecycle guard. A Kill already
    // committed before this poll must win before Stop or Drain reaches children.
    match await_actor_work(
        async {
            state.children.request_all(shutdown);
            state.children.wait_all().await;
        },
        control,
    )
    .await
    {
        Work::Complete(()) => {}
        Work::Killed => return Work::Killed,
        Work::Panicked => return Work::Panicked,
        Work::DropPanicked(()) => return Work::DropPanicked(()),
    }

    await_actor_work(
        async {
            let mut scope = state.stop_scope();
            actor.on_stop(reason, &mut scope).await;
        },
        control,
    )
    .await
}

async fn kill_actor<A: Actor>(
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    // Commit subtree cancellation before running arbitrary Drop code from actor
    // work. Children can then begin terminating even if a destructor is slow.
    inbox.close();
    state.children.request_all(Shutdown::Kill);
    owned.close();
    scheduler.clear(control);
    let mut expected_mode = Mode::Killing;
    loop {
        match close_and_discard(inbox, control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    owned.wait().await;
    state.children.wait_all().await;
    state.children.terminal_status(ExitReason::Killed)
}

async fn fail_actor<A: Actor>(
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    control.begin_failure();
    let reason = match control.mode() {
        Mode::Killing => ExitReason::Killed,
        Mode::Aborting => ExitReason::Aborted,
        Mode::Exited(status) => return status,
        Mode::Running | Mode::Draining | Mode::Stopping | Mode::Failing => ExitReason::Panicked,
    };
    inbox.close();
    state.children.request_all(Shutdown::Kill);
    owned.close();
    scheduler.clear(control);
    let mut expected_mode = control.mode();
    loop {
        match close_and_discard(inbox, control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    owned.wait().await;
    state.children.wait_all().await;
    state.children.terminal_status(reason)
}

const TEARDOWN_DROP_BUDGET: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiscardOutcome {
    Complete,
    ModeChanged,
}

/// Closes the inbox and drops its accepted envelopes in bounded batches.
///
/// Each envelope may run arbitrary user destructors. The lifecycle mode is
/// checked before selecting a value and again after dropping it, so a transition
/// returns [`DiscardOutcome::ModeChanged`] before another value is selected.
/// Every uninterrupted mode pass yields after [`TEARDOWN_DROP_BUDGET`] drops so
/// a large accepted queue cannot monopolize a current-thread executor.
async fn close_and_discard<A: Actor>(
    inbox: &mut ActorInbox<A>,
    control: &Control,
    expected_mode: Mode,
) -> DiscardOutcome {
    inbox.close();
    let mut dropped = 0;
    loop {
        if control.mode() != expected_mode {
            return DiscardOutcome::ModeChanged;
        }

        if !inbox.try_discard() {
            return DiscardOutcome::Complete;
        }

        if control.mode() != expected_mode {
            return DiscardOutcome::ModeChanged;
        }
        dropped += 1;
        if dropped == TEARDOWN_DROP_BUDGET {
            dropped = 0;
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests;
