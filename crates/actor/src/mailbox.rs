use std::{
    any::Any,
    future::{Future, poll_fn},
    panic::{self, AssertUnwindSafe},
    sync::Arc,
    task::{Context, Wake, Waker},
};

use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    Actor, ActorScope, CallError, ExitReason, ExitStatus, Handler, Message, Shutdown,
    ShutdownStatus, owned::OwnedTasks, reply::sealed::HandleReply, scheduler::ReplyScheduler,
};

pub(crate) type ReplyReceiver<R> = oneshot::Receiver<Result<R, CallError>>;

/// A type-erased mailbox entry for one statically checked message.
///
/// Addresses construct a concrete call or one-way envelope only when the actor
/// implements the corresponding `Handler<M>`. Erasure lets one bounded inbox
/// hold every message type handled by that actor. Dynamic dispatch ends at
/// [`Envelope::dispatch`]; the selected handler and reply strategy stay
/// statically dispatched.
pub(crate) type DynEnvelope<A> = Box<dyn Envelope<A>>;

/// Capacity and the concrete envelope remain recoverable when admission loses.
pub(crate) type RejectedAdmission<A, E> = (mpsc::OwnedPermit<DynEnvelope<A>>, Box<E>);

/// Waits without registering an external waker in Tokio's fanout.
///
/// One observer may panic while waking.
/// The proxy contains that panic before Tokio continues fanout.
pub(crate) async fn mode_changed(
    mode: &mut watch::Receiver<Mode>,
) -> Result<(), watch::error::RecvError> {
    let mut changed = std::pin::pin!(mode.changed());
    poll_fn(|task| {
        let waker = Waker::from(Arc::new(PanicSafeWake(task.waker().clone())));
        let mut task = Context::from_waker(&waker);
        changed.as_mut().poll(&mut task)
    })
    .await
}

struct PanicSafeWake(Waker);

impl PanicSafeWake {
    fn forward(&self) {
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| self.0.wake_by_ref())) {
            Control::discard_panic(payload);
        }
    }
}

impl Wake for PanicSafeWake {
    fn wake(self: Arc<Self>) {
        self.forward();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.forward();
    }
}

impl Drop for PanicSafeWake {
    fn drop(&mut self) {
        let waker = std::mem::replace(&mut self.0, Waker::noop().clone());
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(waker))) {
            Control::discard_panic(payload);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// Accepting and dispatching new messages.
    Running,
    /// Dispatching the fixed queue accepted before Drain committed.
    Draining,
    /// Finishing dispatched replies after queued messages were discarded.
    Stopping,
    /// Cooperatively discarding actor work and killing the owned subtree.
    Killing,
    /// Containing an actor panic and killing the owned subtree.
    Failing,
    /// The executor dropped the actor task without awaitable teardown.
    Aborting,
    /// Terminal state and its atomically published status.
    Exited(ExitStatus),
}

/// Owns the actor's authoritative lifecycle state.
///
/// The watch value also gates every lifecycle decision.
/// Admission, dispatch, completion, shutdown, and finalization share its lock.
/// Tokio releases that lock before notifying public observers.
/// Custom public wakers may therefore reenter lifecycle APIs.
/// Private wake hints carry no state.
/// Every runtime consumer rereads [`Mode`].
#[derive(Debug)]
pub(crate) struct Control {
    mode: watch::Sender<Mode>,
    actor_wake: Notify,
    owned_cancellation: CancellationToken,
}

impl Control {
    pub(crate) fn new() -> Arc<Self> {
        let (mode, _) = watch::channel(Mode::Running);

        Arc::new(Self {
            mode,
            actor_wake: Notify::new(),
            owned_cancellation: CancellationToken::new(),
        })
    }

    pub(crate) fn mode(&self) -> Mode {
        *self.mode.borrow()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.mode() == Mode::Running
    }

    pub(crate) fn exit_status(&self) -> Option<ExitStatus> {
        match self.mode() {
            Mode::Exited(status) => Some(status),
            _ => None,
        }
    }

    pub(crate) fn subscribe_mode(&self) -> watch::Receiver<Mode> {
        self.mode.subscribe()
    }

    /// Returns the private signal used to wake owned tasks after hard cutoff.
    ///
    /// [`Mode`] remains authoritative. Tasks also read it before each user poll.
    pub(crate) fn owned_cancellation(&self) -> CancellationToken {
        self.owned_cancellation.clone()
    }

    /// Waits for a private lifecycle hint.
    ///
    /// The actor task must reread [`Mode`] after waking.
    pub(crate) async fn actor_notified(&self) {
        self.actor_wake.notified().await;
    }

    /// Linearizes handler dispatch with graceful cutoff and Kill.
    ///
    /// The returned permit proves dispatch committed before a later lifecycle
    /// transition. No user code or user-owned value is touched under the gate.
    fn begin_dispatch(self: &Arc<Self>) -> Result<DispatchPermit, CallError> {
        self.transact(|mode| {
            let result = if matches!(mode, Mode::Running | Mode::Draining) {
                Ok(DispatchPermit {
                    control: Arc::clone(self),
                })
            } else {
                Err(Self::call_failure_for(mode, CallPhase::Queued))
            };
            (mode, result)
        })
    }

    /// Linearizes a child-exit hook's first entry with lifecycle cutoff.
    ///
    /// The actor task serially waits for each admitted hook.
    /// No running counter is needed.
    /// Stop and Drain wait naturally.
    /// Kill wakes and cancels the hook.
    pub(crate) fn begin_child_hook(&self) -> Option<HookEntryPermit> {
        self.transact(|mode| {
            let permit = (mode == Mode::Running).then_some(HookEntryPermit(()));
            (mode, permit)
        })
    }

    /// Commits the first shutdown mode and permits only a later Kill upgrade.
    ///
    /// Repeated and losing requests observe the already committed behavior;
    /// final actors return their published status.
    pub(crate) fn request(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.transact(|mode| match (mode, shutdown) {
            (Mode::Exited(status), _) => (mode, ShutdownStatus::Exited(status)),
            (Mode::Running, Shutdown::Stop) => (Mode::Stopping, ShutdownStatus::Requested),
            (Mode::Running, Shutdown::Drain) => (Mode::Draining, ShutdownStatus::Requested),
            (Mode::Running, Shutdown::Kill) | (Mode::Draining | Mode::Stopping, Shutdown::Kill) => {
                (Mode::Killing, ShutdownStatus::Requested)
            }
            (Mode::Draining, _) => (mode, ShutdownStatus::InProgress(Shutdown::Drain)),
            (Mode::Stopping, _) => (mode, ShutdownStatus::InProgress(Shutdown::Stop)),
            (Mode::Killing | Mode::Failing | Mode::Aborting, _) => {
                (mode, ShutdownStatus::InProgress(Shutdown::Kill))
            }
        })
    }

    /// Commits panic handling unless Kill, abort, or finalization already won.
    pub(crate) fn begin_failure(&self) {
        self.transact(|mode| {
            let next = if matches!(mode, Mode::Running | Mode::Draining | Mode::Stopping) {
                Mode::Failing
            } else {
                mode
            };
            (next, ())
        });
    }

    pub(crate) fn begin_abort(&self) {
        // ActorTask::drop cannot await descendant teardown.
        // It changes only this actor's reason to Aborted.
        // The final subtree status is always Unconfirmed.
        self.transact(|mode| {
            let next = if matches!(mode, Mode::Exited(_)) {
                mode
            } else {
                Mode::Aborting
            };
            (next, ())
        });
    }

    /// Publishes exactly one terminal mode and returns the status that won.
    ///
    /// Kill or failure may replace only the proposed local reason.
    /// Both preserve the proposed subtree status.
    /// Abort instead forces [`SubtreeStatus::Unconfirmed`](crate::SubtreeStatus::Unconfirmed).
    pub(crate) fn finish(&self, proposed: ExitStatus) -> ExitStatus {
        self.transact(|mode| {
            let reason = match (mode, proposed.reason()) {
                (Mode::Exited(status), _) => return (mode, status),
                (_, ExitReason::Aborted) | (Mode::Aborting, _) => ExitReason::Aborted,
                (Mode::Killing, _) => ExitReason::Killed,
                (Mode::Failing, _) => ExitReason::Panicked,
                (Mode::Running | Mode::Draining | Mode::Stopping, reason) => reason,
            };
            let status = ExitStatus::new(reason, proposed.subtree());
            (Mode::Exited(status), status)
        })
    }

    /// Waits until the lifecycle stream publishes its terminal status.
    pub(crate) async fn wait_for_exit(&self) -> ExitStatus {
        let mut mode = self.subscribe_mode();
        loop {
            if let Mode::Exited(status) = *mode.borrow_and_update() {
                return status;
            }

            mode_changed(&mut mode)
                .await
                .expect("the exit publisher lives until it publishes a status");
        }
    }

    fn call_failure(&self, phase: CallPhase) -> CallError {
        Self::call_failure_for(self.mode(), phase)
    }

    fn call_failure_for(mode: Mode, phase: CallPhase) -> CallError {
        let reason = match mode {
            Mode::Stopping if phase == CallPhase::Queued => ExitReason::Stopped,
            Mode::Killing => ExitReason::Killed,
            Mode::Failing => ExitReason::Panicked,
            Mode::Aborting => ExitReason::Aborted,
            Mode::Exited(status) => status.reason(),
            Mode::Running | Mode::Draining | Mode::Stopping => ExitReason::Panicked,
        };

        match phase {
            CallPhase::Queued => CallError::BeforeDispatch(reason),
            CallPhase::Dispatching => CallError::DuringDispatch(reason),
        }
    }

    /// Commits actor failure and consumes every panic payload without unwinding.
    ///
    /// This is used where another destructor may panic after containment.
    pub(crate) fn contain_panic(&self, payload: Box<dyn Any + Send>) {
        if let Err(notification) = panic::catch_unwind(AssertUnwindSafe(|| self.begin_failure())) {
            Self::discard_panic(notification);
        }
        Self::discard_panic(payload);
    }

    /// Drops one actor-owned user value without unwinding through the runtime.
    pub(crate) fn drop_user_value<T>(&self, value: T) {
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(value))) {
            self.contain_panic(payload);
        }
    }

    /// Notifies one response observer without blaming its Waker on the actor.
    ///
    /// A rejected value remains actor-owned. Its destructor may still fail the
    /// actor through the separate user Drop boundary.
    fn notify_response<T>(&self, reply: oneshot::Sender<T>, value: T) {
        match panic::catch_unwind(AssertUnwindSafe(|| reply.send(value))) {
            Ok(Ok(())) => {}
            Ok(Err(value)) => self.drop_user_value(value),
            Err(payload) => Self::discard_panic(payload),
        }
    }

    fn discard_panic(payload: Box<dyn Any + Send>) {
        if let Err(nested) = panic::catch_unwind(AssertUnwindSafe(|| drop(payload))) {
            // A cascading payload destructor cannot safely leave containment.
            std::mem::forget(nested);
        }
    }

    /// Runs one lifecycle transaction against the sole authoritative value.
    ///
    /// The private actor hint commits before public notification.
    /// Owned cancellation runs after releasing the watch lock.
    /// Their wakers belong only to Tokio tasks.
    /// A public observer panic cannot stall either path.
    fn transact<T>(&self, transaction: impl FnOnce(Mode) -> (Mode, T)) -> T {
        let mut result = None;
        let mut cancel_owned = false;
        let notification = panic::catch_unwind(AssertUnwindSafe(|| {
            self.mode.send_if_modified(|mode| {
                let (next, output) = transaction(*mode);
                let changed = *mode != next;
                result = Some(output);
                *mode = next;
                if changed {
                    self.actor_wake.notify_one();
                    cancel_owned = matches!(next, Mode::Killing | Mode::Failing | Mode::Aborting);
                }
                changed
            });
        }));
        if cancel_owned {
            self.owned_cancellation.cancel();
        }

        match (result, notification) {
            (Some(output), Ok(())) => output,
            (Some(output), Err(payload)) => {
                Self::discard_panic(payload);
                output
            }
            (None, Err(payload)) => panic::resume_unwind(payload),
            (None, Ok(())) => panic!("a watch transaction executes exactly once"),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CallPhase {
    Queued,
    Dispatching,
}

struct DispatchPermit {
    control: Arc<Control>,
}

pub(crate) struct HookEntryPermit(());

struct CompletionPermit;

impl DispatchPermit {
    /// Linearizes successful completion with Kill/failure. The unit permit
    /// carries the decision beyond the transaction without exposing lifecycle
    /// state.
    fn begin_completion(&self) -> Result<CompletionPermit, CallError> {
        self.control.transact(|mode| {
            let result = match mode {
                Mode::Running | Mode::Draining | Mode::Stopping => Ok(CompletionPermit),
                Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_) => {
                    Err(Control::call_failure_for(mode, CallPhase::Dispatching))
                }
            };
            (mode, result)
        })
    }

    /// Commits handler failure before making its dispatch error observable.
    fn fail(&self) -> CallError {
        self.control.begin_failure();
        self.control.call_failure(CallPhase::Dispatching)
    }
}

pub(crate) struct ActorMailbox<A: Actor> {
    pub(crate) sender: mpsc::Sender<DynEnvelope<A>>,
    pub(crate) control: Arc<Control>,
}

impl<A: Actor> ActorMailbox<A> {
    /// Materializes one prebuilt envelope at the admission commit point.
    ///
    /// The fixed operation makes running arbitrary code under the lifecycle
    /// transaction impossible. The private inbox is polled exclusively by the
    /// Tokio-spawned ActorTask, so its wake cannot invoke a user waker. Rejected
    /// permits and envelopes leave through the transaction result and are
    /// returned untouched for lock-free recovery and destruction.
    pub(crate) fn admit<E>(
        &self,
        permit: mpsc::OwnedPermit<DynEnvelope<A>>,
        envelope: Box<E>,
    ) -> Result<mpsc::Sender<DynEnvelope<A>>, RejectedAdmission<A, E>>
    where
        E: Envelope<A> + 'static,
    {
        self.control.transact(move |mode| {
            if mode == Mode::Running {
                (mode, Ok(permit.send(envelope as DynEnvelope<A>)))
            } else {
                (mode, Err((permit, envelope)))
            }
        })
    }

    pub(crate) fn channel(capacity: usize) -> (Arc<Self>, mpsc::Receiver<DynEnvelope<A>>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let control = Control::new();
        (Arc::new(Self { sender, control }), receiver)
    }
}

/// Runtime behavior of a concrete mailbox entry after its message type is
/// erased for storage.
///
/// An admitted entry remains queued until the actor task either discards it or
/// consumes it exactly once for dispatch. Concrete envelope types own their
/// queued cleanup behavior, including whether a waiting caller must be notified.
pub(crate) trait Envelope<A: Actor>: Send {
    /// Attempts to move one accepted entry from the queue into actor execution.
    ///
    /// Abandoned calls return before lifecycle dispatch. Other entries first
    /// commit dispatch against lifecycle shutdown.
    /// If dispatch wins, it invokes the statically selected handler and hands
    /// reply completion to the actor runtime; otherwise it performs queued
    /// rejection without invoking user handler code.
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        owned: &OwnedTasks,
        scheduler: &mut ReplyScheduler<A>,
    );
}

/// Coupled ownership of a two-way call before it leaves the queued phase.
struct QueuedCall<M: Message> {
    message: M,
    reply: oneshot::Sender<Result<M::Reply, CallError>>,
    control: Arc<Control>,
}

enum CallEnvelopeState<M: Message> {
    Queued(QueuedCall<M>),
    /// Tombstone installed after queued ownership leaves the envelope, making
    /// its subsequent `Drop` a no-op.
    Consumed,
}

/// A request-response mailbox entry awaiting dispatch.
///
/// While queued, dropping the caller's response marks the entry abandoned, and
/// dropping the entry reports a phase-aware queued failure to a remaining
/// caller. Dispatch consumes the queued state and transfers reply ownership to
/// [`DispatchReply`].
pub(crate) struct CallEnvelope<M: Message> {
    state: CallEnvelopeState<M>,
}

impl<M: Message> CallEnvelope<M> {
    /// Creates the queued entry and the response endpoint retained by its caller.
    pub(crate) fn new(message: M, control: Arc<Control>) -> (Self, ReplyReceiver<M::Reply>) {
        let (reply, response) = oneshot::channel();
        (
            Self {
                state: CallEnvelopeState::Queued(QueuedCall {
                    message,
                    reply,
                    control,
                }),
            },
            response,
        )
    }

    /// Recovers a message whose envelope lost admission before dispatch.
    pub(crate) fn into_message(mut self) -> M {
        let QueuedCall {
            message,
            reply,
            control,
        } = self.take_queued();
        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(reply))) {
            Control::discard_panic(payload);
        }
        drop(control);
        message
    }

    /// Moves the coupled queued state out while disarming queued-failure Drop.
    fn take_queued(&mut self) -> QueuedCall<M> {
        match std::mem::replace(&mut self.state, CallEnvelopeState::Consumed) {
            CallEnvelopeState::Queued(queued) => queued,
            CallEnvelopeState::Consumed => panic!("a call envelope is consumed at most once"),
        }
    }
}

impl<A, M> Envelope<A> for CallEnvelope<M>
where
    A: Handler<M>,
    M: Message,
{
    fn dispatch(
        mut self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        owned: &OwnedTasks,
        scheduler: &mut ReplyScheduler<A>,
    ) {
        // Only calls can be abandoned; one-way envelopes have no receiver.
        match &self.state {
            CallEnvelopeState::Queued(queued) if queued.reply.is_closed() => return,
            CallEnvelopeState::Queued(_) => {}
            CallEnvelopeState::Consumed => {
                panic!("a consumed call envelope cannot remain in the mailbox")
            }
        }

        let QueuedCall {
            message,
            reply,
            control,
        } = self.take_queued();

        let permit = match control.begin_dispatch() {
            Ok(permit) => permit,
            Err(error) => {
                control.notify_response(reply, Err(error));
                control.drop_user_value(message);
                return;
            }
        };

        // The permit commits DuringDispatch before any user code runs,
        // including synchronous reply construction.
        let reply = DispatchReply::new(reply, permit);
        HandleReply::handle(actor.handle(message, scope), owned, scheduler, reply);
    }
}

impl<M: Message> Drop for CallEnvelope<M> {
    fn drop(&mut self) {
        let CallEnvelopeState::Queued(QueuedCall {
            message,
            reply,
            control,
        }) = std::mem::replace(&mut self.state, CallEnvelopeState::Consumed)
        else {
            return;
        };

        let error = control.call_failure(CallPhase::Queued);
        control.notify_response(reply, Err(error));
        control.drop_user_value(message);
    }
}

/// A queued message whose sender observes admission but not completion.
///
/// Keeping this envelope distinct from `CallEnvelope` makes the absence of a
/// response receiver structural: queued one-way work is never mistaken for an
/// abandoned call and does not allocate a dummy channel.
pub(crate) struct SendEnvelope<M: Message<Reply = ()>> {
    message: M,
    control: Arc<Control>,
}

impl<M: Message<Reply = ()>> SendEnvelope<M> {
    pub(crate) fn new(message: M, control: Arc<Control>) -> Self {
        Self { message, control }
    }

    /// Recovers a message whose envelope lost admission before dispatch.
    pub(crate) fn into_message(self) -> M {
        self.message
    }
}

impl<A, M> Envelope<A> for SendEnvelope<M>
where
    A: Handler<M>,
    M: Message<Reply = ()>,
{
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        owned: &OwnedTasks,
        scheduler: &mut ReplyScheduler<A>,
    ) {
        let Self { message, control } = *self;
        let permit = match control.begin_dispatch() {
            Ok(permit) => permit,
            Err(_) => return,
        };

        // One-way completion still owns a dispatch permit, so panic and Kill
        // use the same state transition as a call even though no result is sent.
        let reply = DispatchReply::one_way(permit);
        HandleReply::handle(actor.handle(message, scope), owned, scheduler, reply);
    }
}

enum DispatchReplyState<R> {
    Caller(oneshot::Sender<Result<R, CallError>>),
    OneWay,
    Completed,
}

pub(crate) struct DispatchReply<R> {
    state: DispatchReplyState<R>,
    permit: DispatchPermit,
}

impl<R> DispatchReply<R> {
    fn new(reply: oneshot::Sender<Result<R, CallError>>, permit: DispatchPermit) -> Self {
        Self {
            state: DispatchReplyState::Caller(reply),
            permit,
        }
    }

    /// Completes dispatched work only if completion commits before Kill,
    /// failure, abort, or terminal publication.
    ///
    /// The gate decides the outcome; caller notification and destruction of a
    /// rejected reply value happen after the watch transaction is released.
    /// One-way work follows the same gate without creating a response channel.
    pub(crate) fn complete(mut self, response: R) {
        let outcome = self.permit.begin_completion();
        let state = std::mem::replace(&mut self.state, DispatchReplyState::Completed);

        match (state, outcome) {
            (DispatchReplyState::Caller(reply), Ok(CompletionPermit)) => {
                self.permit.control.notify_response(reply, Ok(response));
            }
            (DispatchReplyState::Caller(reply), Err(error)) => {
                self.permit.control.notify_response(reply, Err(error));
                self.permit.control.drop_user_value(response);
            }
            (DispatchReplyState::OneWay, Ok(CompletionPermit) | Err(_)) => {
                self.permit.control.drop_user_value(response);
            }
            (DispatchReplyState::Completed, _) => {
                panic!("a dispatch reply completes at most once")
            }
        }
    }
}

impl DispatchReply<()> {
    fn one_way(permit: DispatchPermit) -> Self {
        Self {
            state: DispatchReplyState::OneWay,
            permit,
        }
    }
}

impl<R> Drop for DispatchReply<R> {
    fn drop(&mut self) {
        let state = std::mem::replace(&mut self.state, DispatchReplyState::Completed);
        let reply = match state {
            DispatchReplyState::Caller(reply) => Some(reply),
            DispatchReplyState::OneWay => None,
            DispatchReplyState::Completed => return,
        };
        let error = self.permit.fail();
        if let Some(reply) = reply {
            self.permit.control.notify_response(reply, Err(error));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc as std_mpsc,
        },
        task::{Context, Poll, Wake, Waker},
        time::Duration,
    };

    use crate::{ReplyExt, SubtreeStatus};

    use super::*;

    struct TestActor;

    impl Actor for TestActor {}

    struct NoopEnvelope;

    impl Envelope<TestActor> for NoopEnvelope {
        fn dispatch(
            self: Box<Self>,
            _actor: &mut TestActor,
            _scope: &mut ActorScope<TestActor>,
            _owned: &OwnedTasks,
            _scheduler: &mut ReplyScheduler<TestActor>,
        ) {
        }
    }

    struct RecoverMessage(Arc<AtomicUsize>);

    impl Drop for RecoverMessage {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Message for RecoverMessage {
        type Reply = ();
    }

    impl Handler<RecoverMessage> for TestActor {
        fn handle(
            &mut self,
            _message: RecoverMessage,
            _scope: &mut ActorScope<Self>,
        ) -> impl crate::IntoReply<Self, RecoverMessage> + use<> {
            ().ready()
        }
    }

    // A capacity reservation is not admission. Once Drain wins the lifecycle
    // transaction, the reserved slot must be returned without entering inbox.
    #[tokio::test]
    async fn shutdown_wins_over_an_acquired_but_uncommitted_permit() {
        let (mailbox, mut receiver) = ActorMailbox::<TestActor>::channel(1);
        let permit = mailbox
            .sender
            .clone()
            .reserve_owned()
            .await
            .expect("the test mailbox is open");

        assert_eq!(
            mailbox.control.request(Shutdown::Drain),
            ShutdownStatus::Requested
        );
        let committed = mailbox.admit(permit, Box::new(NoopEnvelope));

        let Err((permit, envelope)) = committed else {
            panic!("shutdown must reject the uncommitted envelope");
        };
        drop(permit);
        drop(envelope);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn committed_send_is_part_of_the_fixed_drain_queue() {
        // Observe the inbox directly to isolate the admission/Drain ordering:
        // once admission wins the shared gate, Drain must retain that envelope.
        let (mailbox, mut receiver) = ActorMailbox::<TestActor>::channel(1);
        let permit = mailbox
            .sender
            .clone()
            .reserve_owned()
            .await
            .expect("the test mailbox is open");

        let committed = mailbox.admit(permit, Box::new(NoopEnvelope));
        let Ok(sender) = committed else {
            panic!("the commit must win admission");
        };
        drop(sender);
        assert_eq!(
            mailbox.control.request(Shutdown::Drain),
            ShutdownStatus::Requested
        );

        assert!(receiver.try_recv().is_ok());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    // The typed rejected path must recover the concrete call message rather
    // than dropping CallEnvelope and publishing a fabricated queued failure.
    #[tokio::test]
    async fn rejected_call_admission_recovers_its_message_without_a_reply() {
        let (mailbox, mut receiver) = ActorMailbox::<TestActor>::channel(1);
        let permit = mailbox
            .sender
            .clone()
            .reserve_owned()
            .await
            .expect("the test mailbox is open");
        let drops = Arc::new(AtomicUsize::new(0));
        let (envelope, mut response) = CallEnvelope::new(
            RecoverMessage(Arc::clone(&drops)),
            Arc::clone(&mailbox.control),
        );
        assert_eq!(
            mailbox.control.request(Shutdown::Stop),
            ShutdownStatus::Requested
        );

        let rejected = mailbox.admit(permit, Box::new(envelope));
        let Err((permit, envelope)) = rejected else {
            panic!("shutdown must reject the call envelope");
        };
        drop(permit);
        let message = (*envelope).into_message();

        assert_eq!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(message);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct PanicWake(Arc<AtomicUsize>);

    impl Wake for PanicWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("intentional notification panic");
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
            panic!("intentional notification panic");
        }
    }

    // Tokio must see a Waker that never unwinds from wake.
    // Direct invocation avoids depending on fanout bucket order.
    #[test]
    fn lifecycle_waker_proxy_contains_external_wake_panic() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let external = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
        let proxy = Waker::from(Arc::new(PanicSafeWake(external)));

        let result = panic::catch_unwind(AssertUnwindSafe(|| proxy.wake_by_ref()));

        assert!(result.is_ok());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    struct PanicWakeDrop(Arc<AtomicBool>);

    impl Wake for PanicWakeDrop {
        fn wake(self: Arc<Self>) {
            panic!("intentional waker panic");
        }

        fn wake_by_ref(self: &Arc<Self>) {
            panic!("intentional waker panic");
        }
    }

    impl Drop for PanicWakeDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            panic!("intentional waker drop panic");
        }
    }

    // Tokio consumes proxy Wakers during fanout.
    // Their inner Waker may also panic while dropping.
    #[test]
    fn lifecycle_waker_proxy_contains_external_drop_panic() {
        let dropped = Arc::new(AtomicBool::new(false));
        let external = Waker::from(Arc::new(PanicWakeDrop(Arc::clone(&dropped))));
        let proxy = Waker::from(Arc::new(PanicSafeWake(external)));

        let result = panic::catch_unwind(AssertUnwindSafe(|| proxy.wake()));

        assert!(result.is_ok());
        assert!(dropped.load(Ordering::SeqCst));
    }

    struct PanicDropMessage(Arc<AtomicBool>);

    impl Message for PanicDropMessage {
        type Reply = ();
    }

    impl Drop for PanicDropMessage {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            panic!("intentional message drop panic");
        }
    }

    // Public observers may install arbitrary safe Wakers.
    // One panic must not suppress another lifecycle waiter.
    // It must not consume the private actor hint.
    #[test]
    fn public_notification_panic_preserves_every_other_waiter() {
        let control = Control::new();
        let public_wakes = Arc::new(AtomicUsize::new(0));
        let public_waker = Waker::from(Arc::new(PanicWake(Arc::clone(&public_wakes))));
        let mut public_task = Context::from_waker(&public_waker);
        let mut public_mode = control.subscribe_mode();
        let mut public_changed = Box::pin(mode_changed(&mut public_mode));
        assert!(public_changed.as_mut().poll(&mut public_task).is_pending());

        let other_wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let other_waker = Waker::from(Arc::clone(&other_wakes));
        let mut other_task = Context::from_waker(&other_waker);
        let mut other_mode = control.subscribe_mode();
        let mut other_changed = Box::pin(mode_changed(&mut other_mode));
        assert!(other_changed.as_mut().poll(&mut other_task).is_pending());

        let actor_wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let actor_waker = Waker::from(Arc::clone(&actor_wakes));
        let mut actor_task = Context::from_waker(&actor_waker);
        let mut actor_notified = Box::pin(control.actor_notified());
        assert!(actor_notified.as_mut().poll(&mut actor_task).is_pending());

        control.begin_failure();

        assert_eq!(public_wakes.load(Ordering::SeqCst), 1);
        assert_eq!(other_wakes.0.load(Ordering::SeqCst), 1);
        assert_eq!(actor_wakes.0.load(Ordering::SeqCst), 1);
        assert!(other_changed.as_mut().poll(&mut other_task).is_ready());
        assert!(actor_notified.as_mut().poll(&mut actor_task).is_ready());
        assert_eq!(control.mode(), Mode::Failing);
    }

    // Admission recovery notifies a still-live response receiver.
    // Its Waker panic must not consume the recovered message.
    // Uncommitted work must not fail the actor.
    #[test]
    fn call_recovery_contains_response_waker_panic() {
        let control = Control::new();
        let message_drops = Arc::new(AtomicUsize::new(0));
        let (envelope, response) = CallEnvelope::new(
            RecoverMessage(Arc::clone(&message_drops)),
            Arc::clone(&control),
        );
        let wakes = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
        let mut task = Context::from_waker(&waker);
        let mut response = Box::pin(response);
        assert!(response.as_mut().poll(&mut task).is_pending());

        let recovered = panic::catch_unwind(AssertUnwindSafe(|| envelope.into_message()));

        assert!(recovered.is_ok());
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(message_drops.load(Ordering::SeqCst), 0);
        assert_eq!(control.mode(), Mode::Running);
        drop(recovered.unwrap());
        assert_eq!(message_drops.load(Ordering::SeqCst), 1);
    }

    // Queued rejection first notifies the caller, then drops its message.
    // Both callbacks may panic and must remain separate containment boundaries.
    #[test]
    fn queued_call_drop_contains_notification_and_message_drop_panics() {
        let control = Control::new();
        let message_dropped = Arc::new(AtomicBool::new(false));
        let (envelope, response) = CallEnvelope::new(
            PanicDropMessage(Arc::clone(&message_dropped)),
            Arc::clone(&control),
        );
        let wakes = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
        let mut task = Context::from_waker(&waker);
        let mut response = Box::pin(response);
        assert!(response.as_mut().poll(&mut task).is_pending());

        let dropped = panic::catch_unwind(AssertUnwindSafe(|| drop(envelope)));

        assert!(dropped.is_ok());
        assert!(message_dropped.load(Ordering::SeqCst));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(control.mode(), Mode::Failing);
        assert!(matches!(
            response.as_mut().get_mut().try_recv(),
            Ok(Err(CallError::BeforeDispatch(ExitReason::Panicked)))
        ));
    }

    // Running-to-Running admission is the hot path. It must take the same write
    // gate without publishing a fake lifecycle change to every closed() waiter.
    #[test]
    fn mailbox_admission_does_not_wake_lifecycle_observers() {
        let (mailbox, mut receiver) = ActorMailbox::<TestActor>::channel(1);
        let mut mode = mailbox.control.subscribe_mode();
        let mut changed = Box::pin(mode.changed());
        let wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&wakes));
        let mut task = Context::from_waker(&waker);
        let permit = mailbox
            .sender
            .clone()
            .try_reserve_owned()
            .expect("the test mailbox has capacity");

        assert!(matches!(changed.as_mut().poll(&mut task), Poll::Pending));
        let admitted = mailbox.admit(permit, Box::new(NoopEnvelope));
        let Ok(sender) = admitted else {
            panic!("Running must admit the envelope");
        };
        drop(sender);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
        assert!(matches!(changed.as_mut().poll(&mut task), Poll::Pending));

        drop(changed);
        assert!(
            !mode
                .has_changed()
                .expect("the control still owns its sender")
        );
        drop(receiver.try_recv().expect("admission physically enqueues"));
    }

    #[test]
    fn kill_before_dispatch_rejects_the_queued_phase() {
        let control = Control::new();
        assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

        assert!(matches!(
            control.begin_dispatch(),
            Err(CallError::BeforeDispatch(ExitReason::Killed))
        ));
    }

    #[tokio::test]
    async fn completion_before_kill_delivers_the_response() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, receiver) = oneshot::channel();

        DispatchReply::new(sender, permit).complete(7_u8);
        assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

        assert_eq!(receiver.await, Ok(Ok(7)));
    }

    // The response value commits before its observer is notified.
    // A panicking observer is not actor code.
    // Its panic must not fail an otherwise healthy actor.
    #[test]
    fn response_waker_panic_does_not_fail_the_actor() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, response) = oneshot::channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
        let mut task = Context::from_waker(&waker);
        let mut response = Box::pin(response);
        assert!(response.as_mut().poll(&mut task).is_pending());

        DispatchReply::new(sender, permit).complete(7_u8);

        assert_eq!(wakes.load(Ordering::SeqCst), 1);
        assert_eq!(response.as_mut().get_mut().try_recv(), Ok(Ok(7)));
        assert_eq!(control.mode(), Mode::Running);
    }

    struct PanicDropReply(Arc<AtomicBool>);

    impl Drop for PanicDropReply {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
            panic!("intentional response drop panic");
        }
    }

    // A closed receiver leaves the response owned by the actor.
    // Its destructor panic must remain contained.
    // The actor must still record that user-code failure.
    #[test]
    fn undelivered_response_drop_panic_fails_the_actor() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, response) = oneshot::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        drop(response);

        DispatchReply::new(sender, permit).complete(PanicDropReply(Arc::clone(&dropped)));

        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), Mode::Failing);
    }

    #[tokio::test]
    async fn kill_before_completion_reports_the_dispatching_phase() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, receiver) = oneshot::channel();
        assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

        DispatchReply::new(sender, permit).complete(7_u8);

        assert!(matches!(
            receiver.await,
            Ok(Err(CallError::DuringDispatch(ExitReason::Killed)))
        ));
    }

    #[tokio::test]
    async fn dropped_dispatch_reply_closes_admission_before_publishing_failure() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, receiver) = oneshot::channel();

        drop(DispatchReply::<()>::new(sender, permit));

        assert!(matches!(
            receiver.await,
            Ok(Err(CallError::DuringDispatch(ExitReason::Panicked)))
        ));
        assert_eq!(control.mode(), Mode::Failing);
        assert!(!control.is_running());
    }

    // Lifecycle and response notifications can invoke safe custom wakers.
    // Neither panic may escape Drop or combine with a later user destructor.
    #[test]
    fn dispatch_reply_drop_contains_notification_panics() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, response) = oneshot::channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
        let mut task = Context::from_waker(&waker);

        let mut mode = control.subscribe_mode();
        let mut changed = Box::pin(mode.changed());
        assert!(changed.as_mut().poll(&mut task).is_pending());
        let mut response = Box::pin(response);
        assert!(response.as_mut().poll(&mut task).is_pending());

        let dropped = panic::catch_unwind(AssertUnwindSafe(|| {
            drop(DispatchReply::<()>::new(sender, permit));
        }));

        assert!(dropped.is_ok());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert_eq!(control.mode(), Mode::Failing);
        assert!(matches!(
            response.as_mut().get_mut().try_recv(),
            Ok(Err(CallError::DuringDispatch(ExitReason::Panicked)))
        ));
    }

    struct GateDropProbe {
        control: Arc<Control>,
        reentered: Arc<AtomicBool>,
    }

    impl Drop for GateDropProbe {
        fn drop(&mut self) {
            let _ = self.control.mode();
            self.reentered.store(true, Ordering::SeqCst);
        }
    }

    // A rejected user response may reenter lifecycle APIs from Drop. Completing
    // on an OS thread turns accidental in-transaction destruction into a bounded
    // failure instead of hanging the entire test process.
    #[tokio::test]
    async fn rejected_response_is_dropped_outside_the_lifecycle_gate() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, receiver) = oneshot::channel();
        let reentered = Arc::new(AtomicBool::new(false));
        assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

        let reply = DispatchReply::new(sender, permit);
        let (done_tx, done_rx) = std_mpsc::sync_channel(1);
        let completion = std::thread::spawn({
            let reentered = Arc::clone(&reentered);
            move || {
                reply.complete(GateDropProbe { control, reentered });
                let _ = done_tx.send(());
            }
        });
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("response Drop must not retain the lifecycle transaction");
        completion.join().expect("completion thread must not panic");

        assert!(matches!(
            receiver.await,
            Ok(Err(CallError::DuringDispatch(ExitReason::Killed)))
        ));
        assert!(reentered.load(Ordering::SeqCst));
    }

    // Kill and panic decide only this actor's local reason.
    // Neither may erase an Unconfirmed descendant guarantee.
    #[test]
    fn hard_mode_precedence_preserves_unconfirmed_subtree() {
        let proposed = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed);

        let killing = Control::new();
        assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
        assert_eq!(
            killing.finish(proposed),
            ExitStatus::new(ExitReason::Killed, SubtreeStatus::Unconfirmed)
        );

        let failing = Control::new();
        failing.begin_failure();
        assert_eq!(
            failing.finish(proposed),
            ExitStatus::new(ExitReason::Panicked, SubtreeStatus::Unconfirmed)
        );

        let exited = Control::new();
        let terminal = ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated);
        assert_eq!(exited.finish(terminal), terminal);
        assert_eq!(
            exited.finish(ExitStatus::new(
                ExitReason::Aborted,
                SubtreeStatus::Unconfirmed,
            )),
            terminal
        );
    }

    // Abort replaces an unfinished Kill or panic publication.
    // It also removes any claimed descendant confirmation.
    #[test]
    fn abort_publication_replaces_unfinished_hard_modes() {
        let aborted = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
        let proposed = ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated);

        let killing = Control::new();
        assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
        killing.begin_abort();
        assert_eq!(killing.mode(), Mode::Aborting);
        assert_eq!(killing.finish(proposed), aborted);

        let failing = Control::new();
        failing.begin_failure();
        failing.begin_abort();
        assert_eq!(failing.mode(), Mode::Aborting);
        assert_eq!(failing.finish(proposed), aborted);

        let exited = Control::new();
        let terminal = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated);
        assert_eq!(exited.finish(terminal), terminal);
        exited.begin_abort();
        assert_eq!(exited.mode(), Mode::Exited(terminal));
        assert_eq!(exited.exit_status(), Some(terminal));
    }
}
