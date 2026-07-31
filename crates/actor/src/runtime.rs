use std::{
    collections::HashMap,
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
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    Actor, ActorRef, Child, ChildExit, ChildId, ErasedFuture, ExitReason, ExitStatus, Shutdown,
    ShutdownStatus, SpawnChildError, SubtreeStatus,
    mailbox::{ActorMailbox, Control, DynEnvelope, HookEntryPermit, Mode},
    owned::OwnedTasks,
    scheduler::ReplyScheduler,
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
/// The returned [`ActorOwner`] uniquely owns the actor lifecycle. The actor runs
/// [`Actor::on_start`] before dispatching its first message. This function must
/// be called from a Tokio runtime.
#[must_use = "dropping the returned owner requests Kill"]
pub fn spawn<A: Actor>(actor: A) -> ActorOwner<A> {
    spawn_with(actor, SpawnOptions::default())
}

/// Spawns a root actor with explicit options.
///
/// The returned [`ActorOwner`] uniquely owns the actor lifecycle. The actor runs
/// [`Actor::on_start`] before dispatching its first message. This function must
/// be called from a Tokio runtime.
#[must_use = "dropping the returned owner requests Kill"]
pub fn spawn_with<A: Actor>(actor: A, options: SpawnOptions) -> ActorOwner<A> {
    let (actor_ref, owned) = spawn_actor(actor, options, None);
    ActorOwner { actor_ref, owned }
}

/// The unique lifecycle owner of a root actor.
///
/// This type is deliberately not cloneable. Dropping it requests a best-effort
/// Kill but cannot synchronously wait from `Drop`; use [`shutdown`](
/// Self::shutdown) or [`wait`](Self::wait) when confirmed normal-path subtree
/// termination matters. [`ExitStatus`] separates the actor's reason from its
/// subtree guarantee.
#[must_use = "dropping an actor owner requests Kill"]
pub struct ActorOwner<A: Actor> {
    actor_ref: ActorRef<A>,
    owned: OwnedActor,
}

impl<A: Actor> ActorOwner<A> {
    /// Returns a cloneable, non-owning address.
    pub fn actor_ref(&self) -> ActorRef<A> {
        self.actor_ref.clone()
    }

    /// Requests Stop, Drain, or Kill without waiting for completion.
    ///
    /// The request atomically closes admission if it establishes a mode. Stop
    /// and Drain are first-wins peers, while Kill may upgrade either one. See
    /// [`Shutdown`] for retained work and cleanup behavior, and
    /// [`ShutdownStatus`] for the meaning of the immediate result.
    pub fn request_shutdown(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.owned.control.request(shutdown)
    }

    /// Returns the terminal status if the actor has already exited.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.owned.control.exit_status()
    }

    /// Waits for the actor to publish its terminal event.
    ///
    /// This method does not initiate shutdown. Keeping `&mut self` allows a
    /// caller to apply an external deadline and upgrade to Kill afterward.
    /// The status's reason describes only this actor.
    /// Its subtree status reports the runtime's termination guarantee.
    pub async fn wait(&mut self) -> ExitStatus {
        self.owned.wait().await
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

impl<A: Actor> fmt::Debug for ActorOwner<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOwner")
            .field("actor_ref", &self.actor_ref)
            .field("exit_status", &self.exit_status())
            .finish_non_exhaustive()
    }
}

/// Runtime capabilities available only while an actor is executing.
///
/// The scope owns child lifecycles on behalf of the actor. Actor state may keep
/// a returned [`Child`] or [`ActorRef`], but those values do not own the child.
/// Graceful shutdown waits for every retained child actor.
/// An unconfirmed descendant remains unconfirmed in the parent's final status.
pub struct ActorScope<A: Actor> {
    actor_ref: ActorRef<A>,
    control: Arc<Control>,
    children: ChildSet,
    // This actor-task-local gate closes only after retained work has finished.
    // Work admitted before graceful cutoff may still add a child that cleanup
    // must include; it is not a second source of shared lifecycle state.
    accepts_children: bool,
    supervisor_tx: mpsc::UnboundedSender<ChildExit>,
}

impl<A: Actor> ActorScope<A> {
    /// Returns this actor's non-owning address.
    ///
    /// An owned reply consumes no interleaved slot.
    /// Its self-call can progress while a slot remains available.
    /// An interleaved reply needs one additional slot for its self-call.
    /// An exclusive reply blocks its queued self-call until it ends.
    ///
    /// A serial lifecycle hook also blocks dispatch. While admission is still
    /// open, as in `on_start` or a running actor's `on_child_exit`, awaiting an
    /// accepted self-call likewise waits until Kill or executor teardown. After
    /// shutdown closes admission, including in `on_stop`, [`ActorRef::call`]
    /// returns [`CallError::Closed`](crate::CallError::Closed) and
    /// [`ActorRef::try_call`] reports
    /// [`TryCallErrorKind::Closed`](crate::TryCallErrorKind::Closed) instead.
    pub const fn myself(&self) -> &ActorRef<A> {
        &self.actor_ref
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
        self.control.request(shutdown)
    }

    /// Spawns a direct child that remains owned by this scope.
    ///
    /// The returned [`Child`] is a non-owning identity and address. Returns the
    /// untouched actor value if post-order cleanup has already closed child
    /// admission.
    pub fn spawn_child<C: Actor>(&mut self, child: C) -> Result<Child<C>, SpawnChildError<C>> {
        self.spawn_child_with(child, SpawnOptions::default())
    }

    /// Spawns a direct child with explicit options that remains owned by this
    /// scope.
    ///
    /// The returned [`Child`] is a non-owning identity and address. Returns the
    /// untouched actor value if post-order cleanup has already closed child
    /// admission.
    pub fn spawn_child_with<C: Actor>(
        &mut self,
        child: C,
        options: SpawnOptions,
    ) -> Result<Child<C>, SpawnChildError<C>> {
        if !self.accepts_children {
            return Err(SpawnChildError::new(child));
        }

        let id = ChildId::new();
        let parent = ParentLink {
            id: id.clone(),
            events: self.supervisor_tx.clone(),
        };
        let (actor_ref, owned) = spawn_actor(child, options, Some(parent));
        self.children.insert(id.clone(), owned);
        Ok(Child::new(id, actor_ref))
    }
}

impl<A: Actor> fmt::Debug for ActorScope<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorScope")
            .field("actor_ref", &self.actor_ref)
            .field("children", &self.children.len())
            .finish_non_exhaustive()
    }
}

struct ParentLink {
    id: ChildId,
    events: mpsc::UnboundedSender<ChildExit>,
}

struct OwnedActor {
    control: Arc<Control>,
    join: Option<JoinHandle<ExitStatus>>,
}

impl OwnedActor {
    async fn wait(&mut self) -> ExitStatus {
        // This bypasses shared lifecycle notification.
        // Cancellation retains the JoinHandle for another wait.
        let joined = match &mut self.join {
            Some(join) => join.await,
            None => return self.control.wait_for_exit().await,
        };
        self.join = None;
        match joined {
            Ok(reason) => reason,
            Err(error) => {
                if let Ok(payload) = error.try_into_panic() {
                    self.control.contain_panic(payload);
                }
                self.control.wait_for_exit().await
            }
        }
    }
}

impl Drop for OwnedActor {
    fn drop(&mut self) {
        if self.control.exit_status().is_none() {
            self.control.request(Shutdown::Kill);
        }
        // Dropping a JoinHandle detaches the task. The Kill request, rather
        // than address liveness, drives cooperative teardown of the subtree.
    }
}

struct ChildSet {
    actors: HashMap<ChildId, OwnedActor>,
    // Removed children cannot erase a lost subtree guarantee.
    subtree: SubtreeStatus,
}

impl Default for ChildSet {
    fn default() -> Self {
        Self {
            actors: HashMap::new(),
            subtree: SubtreeStatus::Terminated,
        }
    }
}

impl ChildSet {
    fn len(&self) -> usize {
        self.actors.len()
    }

    fn insert(&mut self, id: ChildId, actor: OwnedActor) {
        let previous = self.actors.insert(id, actor);
        debug_assert!(previous.is_none(), "child identities are allocation-unique");
    }

    fn remove(&mut self, event: &ChildExit) -> bool {
        if self.actors.remove(event.child()).is_none() {
            return false;
        }
        if event.status().subtree() == SubtreeStatus::Unconfirmed {
            self.subtree = SubtreeStatus::Unconfirmed;
        }
        true
    }

    fn request_all(&self, shutdown: Shutdown) {
        for actor in self.actors.values() {
            actor.control.request(shutdown);
        }
    }

    async fn wait_all(&mut self) {
        // Persist each unconfirmed result before another cancellation point.
        let Self { actors, subtree } = self;
        for actor in actors.values_mut() {
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

fn spawn_actor<A: Actor>(
    actor: A,
    options: SpawnOptions,
    parent: Option<ParentLink>,
) -> (ActorRef<A>, OwnedActor) {
    let (mailbox, inbox) = ActorMailbox::channel(options.mailbox_capacity().get());
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), mailbox.control.subscribe_mode());

    // Each child emits exactly one terminal event. A nonblocking signal plane
    // avoids teardown deadlocks and is bounded by the parent's owned children.
    let (supervisor_tx, supervisor_rx) = mpsc::unbounded_channel();
    let scope = ActorScope {
        actor_ref: actor_ref.clone(),
        control: mailbox.control.clone(),
        children: ChildSet::default(),
        accepts_children: true,
        supervisor_tx,
    };
    let control = mailbox.control.clone();
    let future = Box::pin(run_actor(
        actor,
        scope,
        inbox,
        supervisor_rx,
        mailbox,
        options.max_in_flight(),
    ));
    let task = ActorTask::new(future, ExitGuard::new(control.clone(), parent));
    let join = tokio::spawn(task);

    (
        actor_ref,
        OwnedActor {
            control,
            join: Some(join),
        },
    )
}

struct ExitGuard {
    control: Arc<Control>,
    parent: Option<ParentLink>,
    finished: bool,
}

impl ExitGuard {
    fn new(control: Arc<Control>, parent: Option<ParentLink>) -> Self {
        Self {
            control,
            parent,
            finished: false,
        }
    }

    fn prepare_abort(&self) {
        self.control.begin_abort();
    }

    fn complete(mut self, status: ExitStatus) -> ExitStatus {
        self.publish(status)
    }

    fn publish(&mut self, proposed: ExitStatus) -> ExitStatus {
        if self.finished {
            return self
                .control
                .exit_status()
                .expect("a finished guard published an exit status");
        }
        self.finished = true;
        let status = self.control.finish(proposed);
        if let Some(parent) = &self.parent {
            let _ = parent
                .events
                .send(ChildExit::new(parent.id.clone(), status));
        }
        status
    }
}

impl Drop for ExitGuard {
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

struct ActorTask {
    future: Option<ErasedFuture<'static, ExitStatus>>,
    exit: Option<ExitGuard>,
}

impl ActorTask {
    fn new(future: ErasedFuture<'static, ExitStatus>, exit: ExitGuard) -> Self {
        Self {
            future: Some(future),
            exit: Some(exit),
        }
    }
}

impl Future for ActorTask {
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
        exit.control.drop_user_value(future);
        let status = exit.complete(status);
        Poll::Ready(status)
    }
}

impl Drop for ActorTask {
    fn drop(&mut self) {
        let Some(exit) = self.exit.take() else {
            return;
        };
        // Aborting records a local abort before frame destruction.
        // The guard publishes only after contained cleanup returns.
        exit.prepare_abort();
        if let Some(future) = self.future.take() {
            exit.control.drop_user_value(future);
        }
        drop(exit);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Work {
    Complete,
    Killed,
    Panicked,
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

impl<F> Future for ActorWorkGuard<'_, F>
where
    F: Future<Output = ()>,
{
    type Output = Work;

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
            Ok(Poll::Ready(())) => {
                if self.as_mut().drop_future_panicked() {
                    Poll::Ready(Work::Panicked)
                } else {
                    Poll::Ready(Work::Complete)
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

async fn await_actor_work<F>(future: F, control: &Control) -> Work
where
    F: Future<Output = ()> + Send,
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
    mut actor: A,
    mut scope: ActorScope<A>,
    mut inbox: mpsc::Receiver<DynEnvelope<A>>,
    mut supervisor_rx: mpsc::UnboundedReceiver<ChildExit>,
    _mailbox: Arc<ActorMailbox<A>>,
    max_in_flight: NonZeroUsize,
) -> ExitStatus {
    let control = Arc::clone(&scope.control);
    let owned = OwnedTasks::new(Arc::clone(&scope.control));
    let mut scheduler = ReplyScheduler::new(max_in_flight);
    let mut turn_cursor = TurnCursor::default();

    match await_actor_work(async { actor.on_start(&mut scope).await }, &control).await {
        Work::Complete => {}
        Work::Killed => return kill_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await,
        Work::Panicked => return fail_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await,
    }

    loop {
        match scope.control.mode() {
            Mode::Running => {}
            Mode::Draining => {
                return drain_actor(
                    &mut actor,
                    &mut scope,
                    &mut inbox,
                    &mut supervisor_rx,
                    &control,
                    &owned,
                    &mut scheduler,
                    &mut turn_cursor,
                )
                .await;
            }
            Mode::Stopping => {
                return stop_actor(
                    &mut actor,
                    &mut scope,
                    &mut inbox,
                    &control,
                    &owned,
                    &mut scheduler,
                )
                .await;
            }
            Mode::Killing => {
                return kill_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Exited(status) => return status,
            Mode::Aborting => {
                return ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            }
        }

        let turn = AssertUnwindSafe(actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &control,
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
                return fail_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
            }
        };

        match turn {
            Turn::Mode | Turn::ReplyProgress => {}
            Turn::RepliesFinished => {
                unreachable!("a running actor cannot finish reply scheduling")
            }
            Turn::Child(event) => {
                match handle_child_exit(&mut actor, &mut scope, event, &control).await {
                    Work::Complete => {}
                    Work::Killed => {
                        return kill_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
                    }
                    Work::Panicked => {
                        scope.control.begin_failure();
                        return fail_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
                    }
                }
            }
            Turn::Message => {}
            Turn::InboxClosed => {
                scope.control.begin_failure();
                return fail_actor(&mut scope, &mut inbox, &owned, &mut scheduler).await;
            }
        }
    }
}

enum Turn {
    Mode,
    ReplyProgress,
    // Drain handles child exits while awaiting this barrier.
    RepliesFinished,
    Child(ChildExit),
    Message,
    InboxClosed,
}

#[derive(Default)]
struct TurnCursor {
    ordinary: usize,
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
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    control: &Control,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
    receive_messages: bool,
    expected_mode: Mode,
    cursor: &mut TurnCursor,
) -> Turn {
    let wait_for_owned = !receive_messages && scheduler.is_empty();
    let fair_turn = std::future::poll_fn(|task| {
        if control.mode() != expected_mode {
            return Poll::Ready(Turn::Mode);
        }

        if scheduler.has_exclusive() {
            let ready = scheduler
                .poll_exclusive(actor, scope, control, expected_mode, task)
                .is_ready();
            if control.mode() != expected_mode {
                return Poll::Ready(Turn::Mode);
            }
            return if ready {
                Poll::Ready(Turn::ReplyProgress)
            } else {
                Poll::Pending
            };
        }

        let start = cursor.ordinary;
        for offset in 0..3 {
            if control.mode() != expected_mode {
                return Poll::Ready(Turn::Mode);
            }
            let class = (start + offset) % 3;
            let selected = match class {
                0 if receive_messages && scheduler.has_dispatch_capacity() => {
                    match inbox.poll_recv(task) {
                        Poll::Ready(Some(envelope)) => {
                            envelope.dispatch(actor, scope, owned, scheduler);
                            Some(Turn::Message)
                        }
                        Poll::Ready(None) => Some(Turn::InboxClosed),
                        Poll::Pending => None,
                    }
                }
                1 if scheduler.has_interleaved()
                    && scheduler
                        .poll_interleaved(actor, scope, control, expected_mode, task)
                        .is_ready() =>
                {
                    Some(Turn::ReplyProgress)
                }
                2 => match supervisor_rx.poll_recv(task) {
                    Poll::Ready(Some(event)) => Some(Turn::Child(event)),
                    Poll::Ready(None) => Some(Turn::Mode),
                    Poll::Pending => None,
                },
                _ => None,
            };

            if let Some(turn) = selected {
                cursor.ordinary = (class + 1) % 3;
                return Poll::Ready(turn);
            }
        }

        cursor.ordinary = (start + 1) % 3;
        Poll::Pending
    });

    // Lifecycle always gets first poll rights, especially Kill.
    tokio::select! {
        biased;
        () = control.actor_notified() => Turn::Mode,
        turn = fair_turn => turn,
        () = owned.wait(), if wait_for_owned => Turn::RepliesFinished,
    }
}

async fn handle_child_exit<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    event: ChildExit,
    control: &Control,
) -> Work {
    if !scope.children.remove(&event) {
        return Work::Complete;
    }

    let Some(permit) = scope.control.begin_child_hook() else {
        return Work::Complete;
    };

    run_child_exit_hook(actor, scope, event, control, permit).await
}

/// Makes the private gate proof mandatory at the only user hook call site.
async fn run_child_exit_hook<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    event: ChildExit,
    control: &Control,
    _permit: HookEntryPermit,
) -> Work {
    await_actor_work(async { actor.on_child_exit(event, scope).await }, control).await
}

async fn stop_actor<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    control: &Control,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    match close_and_discard(inbox, &scope.control, Mode::Stopping).await {
        DiscardOutcome::Complete => {}
        DiscardOutcome::ModeChanged => match scope.control.mode() {
            Mode::Killing => return kill_actor(scope, inbox, owned, scheduler).await,
            Mode::Failing => return fail_actor(scope, inbox, owned, scheduler).await,
            // Lifecycle cannot return to a graceful mode. Aborting and Exited
            // belong to ActorTask's outer drop/publication path, which cannot
            // repoll this inner future after committing either state.
            mode => unreachable!("stop discard observed impossible mode: {mode:?}"),
        },
    }
    owned.close();
    match finish_replies(actor, scope, control, owned, scheduler).await {
        Work::Complete => {}
        Work::Killed => return kill_actor(scope, inbox, owned, scheduler).await,
        Work::Panicked => return fail_actor(scope, inbox, owned, scheduler).await,
    }

    match graceful_finish(actor, scope, control, Shutdown::Stop, ExitReason::Stopped).await {
        Work::Complete => scope.children.terminal_status(ExitReason::Stopped),
        Work::Killed => kill_actor(scope, inbox, owned, scheduler).await,
        Work::Panicked => fail_actor(scope, inbox, owned, scheduler).await,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "drain borrows each independent actor-task resource"
)]
async fn drain_actor<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    control: &Control,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
    turn_cursor: &mut TurnCursor,
) -> ExitStatus {
    // Admission physically enqueues under the lifecycle transaction, so this
    // queue is stable once Drain commits. Capacity permits that never reached
    // admission are not accepted work and must not extend graceful shutdown.
    inbox.close();
    let mut inbox_drained = inbox.is_empty();
    if inbox_drained {
        owned.close();
    }
    loop {
        match scope.control.mode() {
            Mode::Killing => {
                return kill_actor(scope, inbox, owned, scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(scope, inbox, owned, scheduler).await;
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
            scope,
            inbox,
            supervisor_rx,
            control,
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
                return fail_actor(scope, inbox, owned, scheduler).await;
            }
        };

        match turn {
            Turn::Mode | Turn::ReplyProgress => {}
            Turn::RepliesFinished => break,
            Turn::Child(event) => match handle_child_exit(actor, scope, event, control).await {
                Work::Complete => {}
                Work::Killed => {
                    return kill_actor(scope, inbox, owned, scheduler).await;
                }
                Work::Panicked => {
                    scope.control.begin_failure();
                    return fail_actor(scope, inbox, owned, scheduler).await;
                }
            },
            Turn::Message => {}
            Turn::InboxClosed => {
                inbox_drained = true;
                owned.close();
            }
        }
    }

    match graceful_finish(actor, scope, control, Shutdown::Drain, ExitReason::Drained).await {
        Work::Complete => scope.children.terminal_status(ExitReason::Drained),
        Work::Killed => kill_actor(scope, inbox, owned, scheduler).await,
        Work::Panicked => fail_actor(scope, inbox, owned, scheduler).await,
    }
}

async fn finish_replies<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    control: &Control,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
) -> Work {
    while !scheduler.is_empty() {
        match scope.control.mode() {
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
                    scheduler.poll_active(actor, scope, control, Mode::Stopping, task)
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
        match scope.control.mode() {
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
            Ok(true) => return Work::Complete,
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
    scope: &mut ActorScope<A>,
    control: &Control,
    shutdown: Shutdown,
    reason: ExitReason,
) -> Work {
    // Cleanup starts after the final child set has been established. Rejecting
    // later spawns keeps the post-order exit guarantee type-visible.
    scope.accepts_children = false;
    // Keep child submission inside the biased lifecycle guard. A Kill already
    // committed before this poll must win before Stop or Drain reaches children.
    match await_actor_work(
        async {
            scope.children.request_all(shutdown);
            scope.children.wait_all().await;
        },
        control,
    )
    .await
    {
        Work::Complete => {}
        Work::Killed => return Work::Killed,
        Work::Panicked => return Work::Panicked,
    }

    await_actor_work(async { actor.on_stop(reason, scope).await }, control).await
}

async fn kill_actor<A: Actor>(
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    // Commit subtree cancellation before running arbitrary Drop code from actor
    // work. Children can then begin terminating even if a destructor is slow.
    inbox.close();
    scope.children.request_all(Shutdown::Kill);
    owned.close();
    scheduler.clear(&scope.control);
    let mut expected_mode = Mode::Killing;
    loop {
        match close_and_discard(inbox, &scope.control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = scope.control.mode(),
        }
    }
    owned.wait().await;
    scope.children.wait_all().await;
    scope.children.terminal_status(ExitReason::Killed)
}

async fn fail_actor<A: Actor>(
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    owned: &OwnedTasks,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitStatus {
    let control = Arc::clone(&scope.control);
    control.begin_failure();
    let reason = match control.mode() {
        Mode::Killing => ExitReason::Killed,
        Mode::Aborting => ExitReason::Aborted,
        Mode::Exited(status) => return status,
        Mode::Running | Mode::Draining | Mode::Stopping | Mode::Failing => ExitReason::Panicked,
    };
    inbox.close();
    scope.children.request_all(Shutdown::Kill);
    owned.close();
    scheduler.clear(&control);
    let mut expected_mode = control.mode();
    loop {
        match close_and_discard(inbox, &control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    owned.wait().await;
    scope.children.wait_all().await;
    scope.children.terminal_status(reason)
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
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    control: &Control,
    expected_mode: Mode,
) -> DiscardOutcome {
    inbox.close();
    let mut dropped = 0;
    loop {
        if control.mode() != expected_mode {
            return DiscardOutcome::ModeChanged;
        }

        let Ok(envelope) = inbox.try_recv() else {
            return DiscardOutcome::Complete;
        };
        control.drop_user_value(envelope);

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
