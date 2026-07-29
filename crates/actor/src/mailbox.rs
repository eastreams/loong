use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    Actor, ActorScope, CallError, ExitReason, Handler, Message, Shutdown, ShutdownStatus,
    reply::sealed::HandleReply, scheduler::ReplyScheduler,
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
    /// Terminal state and its atomically published reason.
    Exited(ExitReason),
}

/// Shared linearization boundary for the complete actor lifecycle.
///
/// The watched value is both the authoritative state and the gate for admission,
/// dispatch, completion, shutdown, and finalization decisions. Transactions use
/// the channel's write lock, while Tokio releases that lock before notifying
/// observers, so a safe custom waker may reenter lifecycle APIs.
#[derive(Debug)]
pub(crate) struct Control {
    mode: watch::Sender<Mode>,
}

impl Control {
    pub(crate) fn new() -> Arc<Self> {
        let (mode, _) = watch::channel(Mode::Running);

        Arc::new(Self { mode })
    }

    pub(crate) fn mode(&self) -> Mode {
        *self.mode.borrow()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.mode() == Mode::Running
    }

    pub(crate) fn exit_reason(&self) -> Option<ExitReason> {
        match self.mode() {
            Mode::Exited(reason) => Some(reason),
            _ => None,
        }
    }

    pub(crate) fn subscribe_mode(&self) -> watch::Receiver<Mode> {
        self.mode.subscribe()
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
    /// The actor task serially waits for an admitted hook, so the permit needs
    /// no running counter: Stop and Drain wait naturally, while Kill may still
    /// cancel the hook through the lifecycle watcher.
    pub(crate) fn begin_child_hook(&self) -> Option<HookEntryPermit> {
        self.transact(|mode| {
            let permit = (mode == Mode::Running).then_some(HookEntryPermit(()));
            (mode, permit)
        })
    }

    /// Commits the first shutdown mode and permits only a later Kill upgrade.
    ///
    /// Repeated and losing requests observe the already committed behavior;
    /// final actors return their published reason.
    pub(crate) fn request(&self, shutdown: Shutdown) -> ShutdownStatus {
        self.transact(|mode| match (mode, shutdown) {
            (Mode::Exited(reason), _) => (mode, ShutdownStatus::Exited(reason)),
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
        // ActorTask::drop cannot await descendant teardown. Even an earlier Kill
        // is therefore downgraded to the explicitly weaker Aborted guarantee
        // unless normal task completion already published the terminal state.
        self.transact(|mode| {
            let next = if matches!(mode, Mode::Exited(_)) {
                mode
            } else {
                Mode::Aborting
            };
            (next, ())
        });
    }

    /// Publishes exactly one terminal mode and returns the reason that won.
    ///
    /// The proposed runner result is accepted only if Kill, failure, or abort has
    /// not already committed through the same gate.
    pub(crate) fn finish(&self, proposed: ExitReason) -> ExitReason {
        self.transact(|mode| {
            // Finalization shares the lifecycle gate with Kill. Whichever
            // commits first determines graceful completion versus escalation.
            let reason = match mode {
                Mode::Killing => ExitReason::Killed,
                Mode::Failing => ExitReason::Panicked,
                Mode::Aborting => ExitReason::Aborted,
                Mode::Running | Mode::Draining | Mode::Stopping => proposed,
                Mode::Exited(reason) => return (mode, reason),
            };
            (Mode::Exited(reason), reason)
        })
    }

    pub(crate) fn fallback_exit_reason(&self) -> ExitReason {
        match self.mode() {
            Mode::Killing => ExitReason::Killed,
            Mode::Failing => ExitReason::Panicked,
            Mode::Exited(reason) => reason,
            Mode::Running | Mode::Draining | Mode::Stopping | Mode::Aborting => ExitReason::Aborted,
        }
    }

    /// Waits on the lifecycle state stream until its terminal reason appears.
    pub(crate) async fn wait_for_exit(&self) -> ExitReason {
        let mut mode = self.subscribe_mode();
        loop {
            if let Mode::Exited(reason) = *mode.borrow_and_update() {
                return reason;
            }

            mode.changed()
                .await
                .expect("the exit publisher lives until it publishes a reason");
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
            Mode::Exited(reason) => reason,
            Mode::Running | Mode::Draining | Mode::Stopping => ExitReason::Panicked,
        };

        match phase {
            CallPhase::Queued => CallError::BeforeDispatch(reason),
            CallPhase::Dispatching => CallError::DuringDispatch(reason),
        }
    }

    /// Runs one lifecycle transaction against the sole authoritative value.
    ///
    /// The callback receives a copy and returns the complete next state, so a
    /// callback panic cannot leave a partially mutated, unnotified mode. Tokio
    /// publishes only actual changes and releases its write lock before waking
    /// observers.
    fn transact<T>(&self, transaction: impl FnOnce(Mode) -> (Mode, T)) -> T {
        let mut result = None;
        self.mode.send_if_modified(|mode| {
            let (next, output) = transaction(*mode);
            let changed = *mode != next;
            result = Some(output);
            *mode = next;
            changed
        });
        result.expect("a watch transaction executes exactly once")
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
        self.control.transact(|mode| {
            let next = if matches!(mode, Mode::Running | Mode::Draining | Mode::Stopping) {
                Mode::Failing
            } else {
                mode
            };
            (
                next,
                Control::call_failure_for(next, CallPhase::Dispatching),
            )
        })
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
    /// reply completion to the actor scheduler; otherwise it performs queued
    /// rejection without invoking user handler code.
    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
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
        drop(reply);
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
                let _ = reply.send(Err(error));
                drop(message);
                return;
            }
        };

        // The permit commits DuringDispatch before any user code runs,
        // including synchronous reply construction.
        let reply = DispatchReply::new(reply, permit);
        HandleReply::handle(actor.handle(message, scope), scheduler, reply);
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

        let _ = reply.send(Err(control.call_failure(CallPhase::Queued)));
        drop(message);
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
        HandleReply::handle(actor.handle(message, scope), scheduler, reply);
    }
}

enum ReplyDestination<R> {
    Caller(oneshot::Sender<Result<R, CallError>>),
    OneWay,
}

enum DispatchReplyState<R> {
    Pending(ReplyDestination<R>),
    Completed,
}

pub(crate) struct DispatchReply<R> {
    state: DispatchReplyState<R>,
    permit: DispatchPermit,
}

impl<R> DispatchReply<R> {
    fn new(reply: oneshot::Sender<Result<R, CallError>>, permit: DispatchPermit) -> Self {
        Self {
            state: DispatchReplyState::Pending(ReplyDestination::Caller(reply)),
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
        let DispatchReplyState::Pending(destination) =
            std::mem::replace(&mut self.state, DispatchReplyState::Completed)
        else {
            panic!("a dispatch reply completes at most once");
        };

        match (destination, outcome) {
            (ReplyDestination::Caller(reply), Ok(CompletionPermit)) => {
                let _ = reply.send(Ok(response));
            }
            (ReplyDestination::Caller(reply), Err(error)) => {
                let _ = reply.send(Err(error));
                drop(response);
            }
            (ReplyDestination::OneWay, Ok(CompletionPermit) | Err(_)) => drop(response),
        }
    }
}

impl DispatchReply<()> {
    fn one_way(permit: DispatchPermit) -> Self {
        Self {
            state: DispatchReplyState::Pending(ReplyDestination::OneWay),
            permit,
        }
    }
}

impl<R> Drop for DispatchReply<R> {
    fn drop(&mut self) {
        let DispatchReplyState::Pending(destination) =
            std::mem::replace(&mut self.state, DispatchReplyState::Completed)
        else {
            return;
        };
        let error = self.permit.fail();
        if let ReplyDestination::Caller(reply) = destination {
            let _ = reply.send(Err(error));
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

    use crate::ReplyExt;

    use super::*;

    struct TestActor;

    impl Actor for TestActor {}

    struct NoopEnvelope;

    impl Envelope<TestActor> for NoopEnvelope {
        fn dispatch(
            self: Box<Self>,
            _actor: &mut TestActor,
            _scope: &mut ActorScope<TestActor>,
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

    #[test]
    fn unfinished_task_teardown_uses_the_weaker_aborted_guarantee() {
        let killing = Control::new();
        assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
        killing.begin_abort();
        assert_eq!(killing.mode(), Mode::Aborting);
        assert_eq!(killing.fallback_exit_reason(), ExitReason::Aborted);

        let failing = Control::new();
        failing.begin_failure();
        failing.begin_abort();
        assert_eq!(failing.mode(), Mode::Aborting);
        assert_eq!(failing.fallback_exit_reason(), ExitReason::Aborted);

        let exited = Control::new();
        assert_eq!(exited.finish(ExitReason::Stopped), ExitReason::Stopped);
        exited.begin_abort();
        assert_eq!(exited.mode(), Mode::Exited(ExitReason::Stopped));
        assert_eq!(exited.exit_reason(), Some(ExitReason::Stopped));
    }
}
