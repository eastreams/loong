use std::{
    collections::HashMap,
    fmt,
    future::Future,
    num::NonZeroUsize,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures_util::FutureExt;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::{
    Actor, ActorRef, Child, ChildExit, ChildId, ErasedFuture, ExitReason, Shutdown, ShutdownStatus,
    SpawnChildError,
    address::wait_for_kill,
    mailbox::{ActorMailbox, Control, DynEnvelope, HookEntryPermit, Mode},
    scheduler::ReplyScheduler,
};

/// Configuration applied when one actor is spawned.
///
/// Mailbox capacity bounds accepted work waiting for dispatch. The independent
/// in-flight limit bounds dispatched replies that have not completed. Both
/// limits default to 32.
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

    /// Sets the maximum number of dispatched, incomplete replies.
    ///
    /// At the limit, new mailbox dispatch pauses while the scheduler continues
    /// polling eligible active replies. The exclusive slot counts toward this
    /// limit, and no slot is reserved for a reply that makes a self-call.
    pub const fn with_max_in_flight(mut self, max_in_flight: NonZeroUsize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    /// Returns the maximum number of queued, not-yet-dequeued messages.
    pub const fn mailbox_capacity(self) -> NonZeroUsize {
        self.mailbox_capacity
    }

    /// Returns the maximum number of dispatched, incomplete replies.
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
/// termination matters. [`ExitReason::Aborted`] explicitly carries a weaker
/// executor-teardown guarantee.
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

    /// Returns the terminal reason if the actor has already exited.
    ///
    /// The reason carries the strong or weak subtree guarantee documented by
    /// [`ExitReason`].
    pub fn exit_reason(&self) -> Option<ExitReason> {
        self.owned.control.exit_reason()
    }

    /// Waits for the actor to publish its terminal event.
    ///
    /// This method does not initiate shutdown. Keeping `&mut self` allows a
    /// caller to apply an external deadline and upgrade to Kill afterward. All
    /// normal exit reasons confirm subtree termination; [`ExitReason::Aborted`]
    /// only confirms that descendant Kill was initiated during task teardown.
    pub async fn wait(&mut self) -> ExitReason {
        self.owned.wait().await
    }

    /// Requests shutdown and waits for the terminal event described by
    /// [`wait`](Self::wait), including its weaker Aborted guarantee.
    ///
    /// The returned reason is the actor's final outcome, which can differ from
    /// the requested mode after a concurrent request, Kill upgrade, panic, or
    /// executor teardown.
    ///
    /// This future owns the actor owner. Cancelling it before terminal
    /// publication therefore drops the owner and requests best-effort Kill. If
    /// Stop or Drain already committed, Drop upgrades it to Kill; if this future
    /// was never polled, the graceful request never committed. To retain control
    /// after cancelling a wait, call [`request_shutdown`](Self::request_shutdown)
    /// and apply the deadline to [`wait`](Self::wait) instead.
    pub async fn shutdown(mut self, shutdown: Shutdown) -> ExitReason {
        self.request_shutdown(shutdown);
        self.wait().await
    }
}

impl<A: Actor> fmt::Debug for ActorOwner<A> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOwner")
            .field("actor_ref", &self.actor_ref)
            .field("exit_reason", &self.exit_reason())
            .finish_non_exhaustive()
    }
}

/// Runtime capabilities available only while an actor is executing.
///
/// The scope owns child lifecycles on behalf of the actor. Actor state may keep
/// a returned [`Child`] or [`ActorRef`], but those values do not own the child.
/// Graceful parent shutdown waits for the owned subtree before the parent's
/// cleanup hook and terminal event.
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
    /// Self-calls from owned and interleaved replies can progress only while an
    /// additional in-flight slot is free. A call accepted from an exclusive
    /// reply cannot be dispatched until that reply ends, so awaiting it requires
    /// Kill, actor failure, or executor teardown to break the wait.
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
    join: Option<JoinHandle<ExitReason>>,
}

impl OwnedActor {
    async fn wait(&mut self) -> ExitReason {
        let reason = self.control.wait_for_exit().await;
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
        reason
    }
}

impl Drop for OwnedActor {
    fn drop(&mut self) {
        if self.control.exit_reason().is_none() {
            self.control.request(Shutdown::Kill);
        }
        // Dropping a JoinHandle detaches the task. The Kill request, rather
        // than address liveness, drives cooperative teardown of the subtree.
    }
}

#[derive(Default)]
struct ChildSet {
    actors: HashMap<ChildId, OwnedActor>,
}

impl ChildSet {
    fn len(&self) -> usize {
        self.actors.len()
    }

    fn insert(&mut self, id: ChildId, actor: OwnedActor) {
        let previous = self.actors.insert(id, actor);
        debug_assert!(previous.is_none(), "child identities are allocation-unique");
    }

    fn remove(&mut self, id: &ChildId) -> bool {
        self.actors.remove(id).is_some()
    }

    fn request_all(&self, shutdown: Shutdown) {
        for actor in self.actors.values() {
            actor.control.request(shutdown);
        }
    }

    async fn wait_all(&mut self) {
        for actor in self.actors.values_mut() {
            actor.wait().await;
        }
        self.actors.clear();
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
    let mode = mailbox.control.subscribe_mode();
    let control = mailbox.control.clone();
    let future = Box::pin(run_actor(
        actor,
        scope,
        inbox,
        supervisor_rx,
        mode,
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

    fn complete(mut self, reason: ExitReason) -> ExitReason {
        self.publish(reason)
    }

    fn publish(&mut self, proposed: ExitReason) -> ExitReason {
        if self.finished {
            return self
                .control
                .exit_reason()
                .expect("a finished guard published an exit reason");
        }
        self.finished = true;
        let reason = self.control.finish(proposed);
        if let Some(parent) = &self.parent {
            let _ = parent
                .events
                .send(ChildExit::new(parent.id.clone(), reason));
        }
        reason
    }
}

impl Drop for ExitGuard {
    fn drop(&mut self) {
        if !self.finished {
            let reason = self.control.fallback_exit_reason();
            let _ = self.publish(reason);
        }
    }
}

struct ActorTask {
    future: Option<ErasedFuture<'static, ExitReason>>,
    exit: Option<ExitGuard>,
}

impl ActorTask {
    fn new(future: ErasedFuture<'static, ExitReason>, exit: ExitGuard) -> Self {
        Self {
            future: Some(future),
            exit: Some(exit),
        }
    }
}

impl Future for ActorTask {
    type Output = ExitReason;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = this
            .future
            .as_mut()
            .expect("completed actor tasks are not polled again")
            .as_mut()
            .poll(context);

        let Poll::Ready(reason) = result else {
            return Poll::Pending;
        };

        // Drop actor state and its ChildSet before publishing this actor's exit.
        drop(this.future.take());
        let reason = this
            .exit
            .take()
            .expect("an actor task owns one exit guard")
            .complete(reason);
        Poll::Ready(reason)
    }
}

impl Drop for ActorTask {
    fn drop(&mut self) {
        if let Some(exit) = &self.exit {
            exit.prepare_abort();
        }
        // Explicit ordering preserves the tree invariant on executor abort:
        // child owners are dropped before the parent exit is published.
        drop(self.future.take());
        drop(self.exit.take());
    }
}

enum Work<T = ()> {
    Complete(T),
    Killed,
    Panicked,
}

async fn await_actor_work<F, T>(future: F, mode: &mut watch::Receiver<Mode>) -> Work<T>
where
    F: Future<Output = T> + Send,
{
    let guarded = AssertUnwindSafe(future).catch_unwind();
    tokio::pin!(guarded);

    tokio::select! {
        biased;
        () = wait_for_kill(mode) => Work::Killed,
        result = &mut guarded => match result {
            Ok(value) => Work::Complete(value),
            Err(_) => Work::Panicked,
        },
    }
}

async fn run_actor<A: Actor>(
    mut actor: A,
    mut scope: ActorScope<A>,
    mut inbox: mpsc::Receiver<DynEnvelope<A>>,
    mut supervisor_rx: mpsc::UnboundedReceiver<ChildExit>,
    mut mode: watch::Receiver<Mode>,
    _mailbox: Arc<ActorMailbox<A>>,
    max_in_flight: NonZeroUsize,
) -> ExitReason {
    let mut scheduler = ReplyScheduler::new(max_in_flight);
    let mut turn_cursor = TurnCursor::default();

    match await_actor_work(async { actor.on_start(&mut scope).await }, &mut mode).await {
        Work::Complete(()) => {}
        Work::Killed => return kill_actor(&mut scope, &mut inbox, &mut scheduler).await,
        Work::Panicked => return fail_actor(&mut scope, &mut inbox, &mut scheduler).await,
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
                    &mut mode,
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
                    &mut mode,
                    &mut scheduler,
                )
                .await;
            }
            Mode::Killing => {
                return kill_actor(&mut scope, &mut inbox, &mut scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(&mut scope, &mut inbox, &mut scheduler).await;
            }
            Mode::Exited(reason) => return reason,
            Mode::Aborting => return ExitReason::Aborted,
        }

        let turn = AssertUnwindSafe(actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &mut mode,
            &mut scheduler,
            true,
            Mode::Running,
            &mut turn_cursor,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(_) => {
                scope.control.begin_failure();
                return fail_actor(&mut scope, &mut inbox, &mut scheduler).await;
            }
        };

        match turn {
            Turn::Mode | Turn::ReplyProgress => {}
            Turn::Child(event) => {
                match handle_child_exit(&mut actor, &mut scope, event, &mut mode).await {
                    Work::Complete(()) => {}
                    Work::Killed => {
                        return kill_actor(&mut scope, &mut inbox, &mut scheduler).await;
                    }
                    Work::Panicked => {
                        scope.control.begin_failure();
                        return fail_actor(&mut scope, &mut inbox, &mut scheduler).await;
                    }
                }
            }
            Turn::Message => {}
            Turn::InboxClosed => {
                scope.control.begin_failure();
                return fail_actor(&mut scope, &mut inbox, &mut scheduler).await;
            }
        }
    }
}

enum Turn {
    Mode,
    ReplyProgress,
    Child(ChildExit),
    Message,
    InboxClosed,
}

#[derive(Default)]
struct TurnCursor {
    ordinary: usize,
    exclusive_owned_first: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "one turn borrows each independently owned runner resource"
)]
// Ordinary turns resume from a persistent cursor across mailbox dispatch,
// owned replies, interleaved replies, and child-exit events. With exclusive work
// present, only owned and exclusive replies are eligible, and their first-poll
// priority alternates. Lifecycle observation sits outside this fairness domain
// with biased priority, and mode checks between candidates keep Kill ahead of
// subsequent user polls.
async fn actor_turn<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    mode: &mut watch::Receiver<Mode>,
    scheduler: &mut ReplyScheduler<A>,
    receive_messages: bool,
    expected_mode: Mode,
    cursor: &mut TurnCursor,
) -> Turn {
    let control = Arc::clone(&scope.control);
    let fair_turn = std::future::poll_fn(|task| {
        if control.mode() != expected_mode {
            return Poll::Ready(Turn::Mode);
        }

        if scheduler.has_exclusive() {
            let owned_first = cursor.exclusive_owned_first;
            for poll_owned in [owned_first, !owned_first] {
                if control.mode() != expected_mode {
                    return Poll::Ready(Turn::Mode);
                }
                let ready = if poll_owned {
                    scheduler.has_owned()
                        && scheduler
                            .poll_owned(&control, expected_mode, task)
                            .is_ready()
                } else {
                    scheduler
                        .poll_exclusive(actor, scope, &control, expected_mode, task)
                        .is_ready()
                };
                if ready {
                    cursor.exclusive_owned_first = !poll_owned;
                    return Poll::Ready(Turn::ReplyProgress);
                }
            }
            cursor.exclusive_owned_first = !owned_first;
            return Poll::Pending;
        }

        let start = cursor.ordinary;
        for offset in 0..4 {
            if control.mode() != expected_mode {
                return Poll::Ready(Turn::Mode);
            }
            let class = (start + offset) % 4;
            let selected = match class {
                0 if receive_messages && scheduler.can_dispatch() => match inbox.poll_recv(task) {
                    Poll::Ready(Some(envelope)) => {
                        start_envelope(actor, scope, scheduler, envelope);
                        Some(Turn::Message)
                    }
                    Poll::Ready(None) => Some(Turn::InboxClosed),
                    Poll::Pending => None,
                },
                1 if scheduler.has_owned()
                    && scheduler
                        .poll_owned(&control, expected_mode, task)
                        .is_ready() =>
                {
                    Some(Turn::ReplyProgress)
                }
                2 if scheduler.has_interleaved()
                    && scheduler
                        .poll_interleaved(actor, scope, &control, expected_mode, task)
                        .is_ready() =>
                {
                    Some(Turn::ReplyProgress)
                }
                3 => match supervisor_rx.poll_recv(task) {
                    Poll::Ready(Some(event)) => Some(Turn::Child(event)),
                    Poll::Ready(None) => Some(Turn::Mode),
                    Poll::Pending => None,
                },
                _ => None,
            };

            if let Some(turn) = selected {
                cursor.ordinary = (class + 1) % 4;
                return Poll::Ready(turn);
            }
        }

        cursor.ordinary = (start + 1) % 4;
        Poll::Pending
    });

    // Keep the watch future alive while the fair-turn future is pending. The
    // biased order gives lifecycle control, especially Kill, first poll rights.
    tokio::select! {
        biased;
        _ = mode.changed() => Turn::Mode,
        turn = fair_turn => turn,
    }
}

/// Starts user-visible dispatch only after admission and capacity checks. This
/// is the sole transition from a queued envelope into scheduler-owned work.
fn start_envelope<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    scheduler: &mut ReplyScheduler<A>,
    envelope: DynEnvelope<A>,
) {
    if envelope.is_abandoned() {
        return;
    }

    envelope.dispatch(actor, scope, scheduler);
}

async fn handle_child_exit<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    event: ChildExit,
    mode: &mut watch::Receiver<Mode>,
) -> Work {
    if !scope.children.remove(event.child()) {
        return Work::Complete(());
    }

    let Some(permit) = scope.control.begin_child_hook() else {
        return Work::Complete(());
    };

    run_child_exit_hook(actor, scope, event, mode, permit).await
}

/// Makes the private gate proof mandatory at the only user hook call site.
async fn run_child_exit_hook<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    event: ChildExit,
    mode: &mut watch::Receiver<Mode>,
    _permit: HookEntryPermit,
) -> Work {
    await_actor_work(async { actor.on_child_exit(event, scope).await }, mode).await
}

async fn stop_actor<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    mode: &mut watch::Receiver<Mode>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitReason {
    match close_and_discard(inbox, &scope.control, Mode::Stopping).await {
        DiscardOutcome::Complete => {}
        DiscardOutcome::ModeChanged => match scope.control.mode() {
            Mode::Killing => return kill_actor(scope, inbox, scheduler).await,
            Mode::Failing => return fail_actor(scope, inbox, scheduler).await,
            // Lifecycle cannot return to a graceful mode. Aborting and Exited
            // belong to ActorTask's outer drop/publication path, which cannot
            // repoll this inner future after committing either state.
            mode => unreachable!("stop discard observed impossible mode: {mode:?}"),
        },
    }
    match finish_replies(actor, scope, scheduler, mode).await {
        Work::Complete(()) => {}
        Work::Killed => return kill_actor(scope, inbox, scheduler).await,
        Work::Panicked => return fail_actor(scope, inbox, scheduler).await,
    }

    match graceful_finish(actor, scope, Shutdown::Stop, ExitReason::Stopped, mode).await {
        Work::Complete(()) => ExitReason::Stopped,
        Work::Killed => kill_actor(scope, inbox, scheduler).await,
        Work::Panicked => fail_actor(scope, inbox, scheduler).await,
    }
}

async fn drain_actor<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    supervisor_rx: &mut mpsc::UnboundedReceiver<ChildExit>,
    mode: &mut watch::Receiver<Mode>,
    scheduler: &mut ReplyScheduler<A>,
    turn_cursor: &mut TurnCursor,
) -> ExitReason {
    // Admission physically enqueues under the lifecycle transaction, so this
    // queue is stable once Drain commits. Capacity permits that never reached
    // admission are not accepted work and must not extend graceful shutdown.
    inbox.close();
    mode.borrow_and_update();
    let mut inbox_drained = inbox.is_empty();
    loop {
        match scope.control.mode() {
            Mode::Killing => {
                return kill_actor(scope, inbox, scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(scope, inbox, scheduler).await;
            }
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Exited(reason) => return reason,
            Mode::Aborting => return ExitReason::Aborted,
        }

        if !inbox_drained && inbox.is_empty() {
            inbox_drained = true;
        }

        if inbox_drained && scheduler.is_empty() {
            let Ok(event) = supervisor_rx.try_recv() else {
                break;
            };
            match handle_child_exit(actor, scope, event, mode).await {
                Work::Complete(()) => continue,
                Work::Killed => return kill_actor(scope, inbox, scheduler).await,
                Work::Panicked => {
                    scope.control.begin_failure();
                    return fail_actor(scope, inbox, scheduler).await;
                }
            }
        }

        let turn = AssertUnwindSafe(actor_turn(
            actor,
            scope,
            inbox,
            supervisor_rx,
            mode,
            scheduler,
            !inbox_drained,
            Mode::Draining,
            turn_cursor,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(_) => {
                scope.control.begin_failure();
                return fail_actor(scope, inbox, scheduler).await;
            }
        };

        match turn {
            Turn::Mode | Turn::ReplyProgress => {}
            Turn::Child(event) => match handle_child_exit(actor, scope, event, mode).await {
                Work::Complete(()) => {}
                Work::Killed => {
                    return kill_actor(scope, inbox, scheduler).await;
                }
                Work::Panicked => {
                    scope.control.begin_failure();
                    return fail_actor(scope, inbox, scheduler).await;
                }
            },
            Turn::Message => {}
            Turn::InboxClosed => inbox_drained = true,
        }
    }

    match graceful_finish(actor, scope, Shutdown::Drain, ExitReason::Drained, mode).await {
        Work::Complete(()) => ExitReason::Drained,
        Work::Killed => kill_actor(scope, inbox, scheduler).await,
        Work::Panicked => fail_actor(scope, inbox, scheduler).await,
    }
}

async fn finish_replies<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    scheduler: &mut ReplyScheduler<A>,
    mode: &mut watch::Receiver<Mode>,
) -> Work {
    mode.borrow_and_update();
    let control = Arc::clone(&scope.control);
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
                _ = mode.changed() => {}
                () = std::future::poll_fn(|task| {
                    scheduler.poll_active(actor, scope, &control, Mode::Stopping, task)
                }) => {}
            }
        })
        .catch_unwind()
        .await;

        if result.is_err() {
            scope.control.begin_failure();
            return Work::Panicked;
        }
    }

    Work::Complete(())
}

/// Graceful shutdown is post-order: a parent finishes the work retained by the
/// selected mode, then waits for children, and only then runs its cleanup hook.
async fn graceful_finish<A: Actor>(
    actor: &mut A,
    scope: &mut ActorScope<A>,
    shutdown: Shutdown,
    reason: ExitReason,
    mode: &mut watch::Receiver<Mode>,
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
        mode,
    )
    .await
    {
        Work::Complete(()) => {}
        Work::Killed => return Work::Killed,
        Work::Panicked => return Work::Panicked,
    }

    await_actor_work(async { actor.on_stop(reason, scope).await }, mode).await
}

async fn kill_actor<A: Actor>(
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitReason {
    // Commit subtree cancellation before running arbitrary Drop code from actor
    // work. Children can then begin terminating even if a destructor is slow.
    inbox.close();
    scope.children.request_all(Shutdown::Kill);
    scheduler.clear();
    let mut expected_mode = Mode::Killing;
    loop {
        match close_and_discard(inbox, &scope.control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = scope.control.mode(),
        }
    }
    scope.children.wait_all().await;
    ExitReason::Killed
}

async fn fail_actor<A: Actor>(
    scope: &mut ActorScope<A>,
    inbox: &mut mpsc::Receiver<DynEnvelope<A>>,
    scheduler: &mut ReplyScheduler<A>,
) -> ExitReason {
    let control = Arc::clone(&scope.control);
    control.begin_failure();
    let reason = match control.mode() {
        Mode::Killing => ExitReason::Killed,
        Mode::Aborting => ExitReason::Aborted,
        Mode::Exited(reason) => reason,
        Mode::Running | Mode::Draining | Mode::Stopping | Mode::Failing => ExitReason::Panicked,
    };
    inbox.close();
    scope.children.request_all(Shutdown::Kill);
    scheduler.clear();
    let mut expected_mode = control.mode();
    loop {
        match close_and_discard(inbox, &control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    scope.children.wait_all().await;
    reason
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
        drop(envelope);

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
