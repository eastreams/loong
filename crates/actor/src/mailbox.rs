use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    Actor, ActorScope, CallError, ExitReason, Handler, Message, Shutdown, ShutdownStatus,
    reply::sealed::HandleReply, scheduler::ReplyScheduler,
};

pub(crate) type ReplyReceiver<R> = oneshot::Receiver<Result<R, CallError>>;
pub(crate) type DynEnvelope<A> = Box<dyn Envelope<A>>;

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
/// The mutex-protected mode is authoritative for admission, dispatch,
/// completion, shutdown, and finalization decisions. `mode_tx` mirrors every
/// committed transition while that gate is held, giving async observers one
/// state stream whose terminal variant always carries its [`ExitReason`].
#[derive(Debug)]
pub(crate) struct Control {
    gate: Mutex<Mode>,
    mode_tx: watch::Sender<Mode>,
}

impl Control {
    pub(crate) fn new() -> Arc<Self> {
        let (mode_tx, _) = watch::channel(Mode::Running);

        Arc::new(Self {
            gate: Mutex::new(Mode::Running),
            mode_tx,
        })
    }

    pub(crate) fn mode(&self) -> Mode {
        *self.lock_gate()
    }

    pub(crate) fn is_running(&self) -> bool {
        self.mode() == Mode::Running
    }

    pub(crate) fn exit_reason(&self) -> Option<ExitReason> {
        match *self.lock_gate() {
            Mode::Exited(reason) => Some(reason),
            _ => None,
        }
    }

    pub(crate) fn subscribe_mode(&self) -> watch::Receiver<Mode> {
        self.mode_tx.subscribe()
    }

    /// Serializes request commit with lifecycle cutoff.
    ///
    /// The closure runs exactly once only while Running. It must not await or
    /// invoke user code.
    pub(crate) fn admit<T>(&self, commit: impl FnOnce() -> T) -> Result<T, ()> {
        let gate = self.lock_gate();
        if *gate != Mode::Running {
            return Err(());
        }

        Ok(commit())
    }

    /// Linearizes handler dispatch with graceful cutoff and Kill.
    ///
    /// The returned permit proves dispatch committed before a later lifecycle
    /// transition. No user code or user-owned value is touched under the gate.
    fn begin_dispatch(self: &Arc<Self>) -> Result<DispatchPermit, CallError> {
        let gate = self.lock_gate();
        if matches!(*gate, Mode::Running | Mode::Draining) {
            Ok(DispatchPermit {
                control: Arc::clone(self),
            })
        } else {
            Err(Self::call_failure_locked(*gate, CallPhase::Queued))
        }
    }

    /// Commits the first shutdown mode and permits only a later Kill upgrade.
    ///
    /// Repeated and losing requests observe the already committed behavior;
    /// final actors return their published reason.
    pub(crate) fn request(&self, shutdown: Shutdown) -> ShutdownStatus {
        let mut gate = self.lock_gate();

        let next = match (*gate, shutdown) {
            (Mode::Exited(reason), _) => return ShutdownStatus::Exited(reason),
            (Mode::Running, Shutdown::Stop) => Mode::Stopping,
            (Mode::Running, Shutdown::Drain) => Mode::Draining,
            (Mode::Running, Shutdown::Kill) => Mode::Killing,
            (Mode::Draining | Mode::Stopping, Shutdown::Kill) => Mode::Killing,
            (Mode::Draining, _) => return ShutdownStatus::InProgress(Shutdown::Drain),
            (Mode::Stopping, _) => return ShutdownStatus::InProgress(Shutdown::Stop),
            (Mode::Killing | Mode::Failing | Mode::Aborting, _) => {
                return ShutdownStatus::InProgress(Shutdown::Kill);
            }
        };

        self.set_mode(&mut gate, next);
        ShutdownStatus::Requested
    }

    /// Commits panic handling unless Kill, abort, or finalization already won.
    pub(crate) fn begin_failure(&self) {
        let mut gate = self.lock_gate();
        self.begin_failure_locked(&mut gate);
    }

    // Both caught panics and DispatchReply's pre-publication path use this
    // transition so Kill/abort precedence cannot diverge between them.
    fn begin_failure_locked(&self, gate: &mut Mode) {
        if matches!(*gate, Mode::Running | Mode::Draining | Mode::Stopping) {
            self.set_mode(gate, Mode::Failing);
        }
    }

    pub(crate) fn begin_abort(&self) {
        let mut gate = self.lock_gate();
        // ActorTask::drop cannot await descendant teardown. Even an earlier Kill
        // is therefore downgraded to the explicitly weaker Aborted guarantee
        // unless normal task completion already published the terminal state.
        if !matches!(*gate, Mode::Exited(_)) {
            self.set_mode(&mut gate, Mode::Aborting);
        }
    }

    /// Publishes exactly one terminal mode and returns the reason that won.
    ///
    /// The proposed runner result is accepted only if Kill, failure, or abort has
    /// not already committed through the same gate.
    pub(crate) fn finish(&self, proposed: ExitReason) -> ExitReason {
        let mut gate = self.lock_gate();
        if let Mode::Exited(reason) = *gate {
            return reason;
        }

        // Finalization shares the lifecycle gate with Kill. Whichever commits
        // first determines whether graceful completion or escalation wins.
        let reason = match *gate {
            Mode::Killing => ExitReason::Killed,
            Mode::Failing => ExitReason::Panicked,
            Mode::Aborting => ExitReason::Aborted,
            Mode::Running | Mode::Draining | Mode::Stopping => proposed,
            Mode::Exited(_) => unreachable!("exited was handled above"),
        };
        self.set_mode(&mut gate, Mode::Exited(reason));
        reason
    }

    pub(crate) fn fallback_exit_reason(&self) -> ExitReason {
        match *self.lock_gate() {
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
        let gate = self.lock_gate();
        Self::call_failure_locked(*gate, phase)
    }

    fn call_failure_locked(mode: Mode, phase: CallPhase) -> CallError {
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

    fn set_mode(&self, gate: &mut Mode, mode: Mode) {
        *gate = mode;
        // Publish while holding the admission gate so concurrent requests can
        // never make the observed lifecycle move backwards.
        let _ = self.mode_tx.send_replace(mode);
    }

    fn lock_gate(&self) -> MutexGuard<'_, Mode> {
        // No user code runs under this mutex. Recovering poison preserves the
        // terminal state machine if an internal assertion ever unwinds.
        self.gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

struct CompletionPermit;

impl DispatchPermit {
    /// Linearizes a successful response with Kill/failure. The unit permit
    /// carries the decision beyond the mutex without exposing lifecycle state.
    fn begin_completion(&self) -> Result<CompletionPermit, CallError> {
        let gate = self.control.lock_gate();
        match *gate {
            Mode::Running | Mode::Draining | Mode::Stopping => Ok(CompletionPermit),
            Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_) => {
                Err(Control::call_failure_locked(*gate, CallPhase::Dispatching))
            }
        }
    }

    /// Commits handler failure before making its dispatch error observable.
    fn fail(&self) -> CallError {
        let mut gate = self.control.lock_gate();
        self.control.begin_failure_locked(&mut gate);
        Control::call_failure_locked(*gate, CallPhase::Dispatching)
    }
}

pub(crate) struct ActorMailbox<A: Actor> {
    pub(crate) sender: mpsc::Sender<DynEnvelope<A>>,
    pub(crate) control: Arc<Control>,
}

impl<A: Actor> ActorMailbox<A> {
    pub(crate) fn channel(capacity: usize) -> (Arc<Self>, mpsc::Receiver<DynEnvelope<A>>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let control = Control::new();
        (Arc::new(Self { sender, control }), receiver)
    }
}

pub(crate) trait Envelope<A: Actor>: Send {
    fn is_abandoned(&self) -> bool;

    fn dispatch(
        self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        scheduler: &mut ReplyScheduler<A>,
    );
}

pub(crate) struct CallEnvelope<M: Message> {
    message: Option<M>,
    reply: Option<oneshot::Sender<Result<M::Reply, CallError>>>,
    control: Arc<Control>,
}

impl<M: Message> CallEnvelope<M> {
    pub(crate) fn new(message: M, control: Arc<Control>) -> (Self, ReplyReceiver<M::Reply>) {
        let (reply, response) = oneshot::channel();
        (
            Self {
                message: Some(message),
                reply: Some(reply),
                control,
            },
            response,
        )
    }
}

impl<A, M> Envelope<A> for CallEnvelope<M>
where
    A: Handler<M>,
    M: Message,
{
    fn is_abandoned(&self) -> bool {
        self.reply.as_ref().is_none_or(oneshot::Sender::is_closed)
    }

    fn dispatch(
        mut self: Box<Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        scheduler: &mut ReplyScheduler<A>,
    ) {
        let message = self
            .message
            .take()
            .expect("an envelope is dispatched at most once");
        let reply = self
            .reply
            .take()
            .expect("an envelope is dispatched at most once");
        let control = Arc::clone(&self.control);

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
        let Some(reply) = self.reply.take() else {
            return;
        };
        let _ = reply.send(Err(self.control.call_failure(CallPhase::Queued)));
    }
}

pub(crate) struct DispatchReply<R> {
    reply: Option<oneshot::Sender<Result<R, CallError>>>,
    permit: DispatchPermit,
}

impl<R> DispatchReply<R> {
    fn new(reply: oneshot::Sender<Result<R, CallError>>, permit: DispatchPermit) -> Self {
        Self {
            reply: Some(reply),
            permit,
        }
    }

    /// Completes the call only if its response commits before Kill, failure,
    /// abort, or terminal publication.
    ///
    /// The gate decides the outcome; channel notification and destruction of a
    /// rejected user response happen after the mutex is released.
    pub(crate) fn complete(mut self, response: R) {
        let outcome = self.permit.begin_completion();
        let reply = self
            .reply
            .take()
            .expect("a dispatch reply completes at most once");

        match outcome {
            Ok(CompletionPermit) => {
                let _ = reply.send(Ok(response));
            }
            Err(error) => {
                let _ = reply.send(Err(error));
                drop(response);
            }
        }
    }
}

impl<R> Drop for DispatchReply<R> {
    fn drop(&mut self) {
        let Some(reply) = self.reply.take() else {
            return;
        };
        let error = self.permit.fail();
        let _ = reply.send(Err(error));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct TestActor;

    impl Actor for TestActor {}

    struct NoopEnvelope;

    impl Envelope<TestActor> for NoopEnvelope {
        fn is_abandoned(&self) -> bool {
            false
        }

        fn dispatch(
            self: Box<Self>,
            _actor: &mut TestActor,
            _scope: &mut ActorScope<TestActor>,
            _scheduler: &mut ReplyScheduler<TestActor>,
        ) {
        }
    }

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
        let committed = mailbox.control.admit(|| {
            drop(permit.send(Box::new(NoopEnvelope)));
        });

        assert_eq!(committed, Err(()));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn committed_send_is_part_of_the_fixed_drain_queue() {
        let (mailbox, mut receiver) = ActorMailbox::<TestActor>::channel(1);
        let permit = mailbox
            .sender
            .clone()
            .reserve_owned()
            .await
            .expect("the test mailbox is open");

        mailbox
            .control
            .admit(|| {
                drop(permit.send(Box::new(NoopEnvelope)));
            })
            .expect("the commit wins admission");
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
        assert_eq!(control.admit(|| ()), Err(()));
    }

    struct GateDropProbe {
        control: Arc<Control>,
        dropped_without_gate: Arc<AtomicBool>,
    }

    impl Drop for GateDropProbe {
        fn drop(&mut self) {
            self.dropped_without_gate
                .store(self.control.gate.try_lock().is_ok(), Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn rejected_response_is_dropped_outside_the_lifecycle_gate() {
        let control = Control::new();
        let permit = control.begin_dispatch().expect("dispatch wins the gate");
        let (sender, receiver) = oneshot::channel();
        let dropped_without_gate = Arc::new(AtomicBool::new(false));
        assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

        DispatchReply::new(sender, permit).complete(GateDropProbe {
            control,
            dropped_without_gate: Arc::clone(&dropped_without_gate),
        });

        assert!(matches!(
            receiver.await,
            Ok(Err(CallError::DuringDispatch(ExitReason::Killed)))
        ));
        assert!(dropped_without_gate.load(Ordering::SeqCst));
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
