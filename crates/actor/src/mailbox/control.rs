//! Lifecycle state and unforgeable dispatch permits.
//!
//! Mailbox admission is implemented here.
//! Raw transactions remain private to this module.

use std::{
    any::Any,
    future::{Future, poll_fn},
    panic::{self, AssertUnwindSafe},
    sync::Arc,
    task::{Context, Wake, Waker},
};

use tokio::sync::{Notify, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::{Actor, CallError, ExitReason, ExitStatus, Shutdown, ShutdownStatus};

use super::{ActorMailbox, DynEnvelope, Envelope, RejectedAdmission};

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
    pub(super) fn begin_dispatch(self: &Arc<Self>) -> Result<DispatchPermit, CallError> {
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
    /// Abort instead forces an unconfirmed subtree status.
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

    /// Reports failure for work that never left mailbox ownership.
    pub(super) fn queued_failure(&self) -> CallError {
        self.call_failure(CallPhase::Queued)
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

    pub(super) fn discard_panic(payload: Box<dyn Any + Send>) {
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

impl<A: Actor> ActorMailbox<A> {
    /// Commits a reserved mailbox slot while admission remains open.
    ///
    /// `Ok(())` means the envelope entered the mailbox.
    /// Later shutdown may still discard it.
    ///
    /// Closed admission returns the permit and original envelope unchanged.
    /// The caller then releases capacity and recovers the message.
    /// Both actions happen after the transaction ends.
    ///
    /// Only [`mpsc::Permit::send`] runs inside the transaction.
    /// It can wake only the private actor task.
    /// No user callback or destructor runs inside it.
    pub(crate) fn admit<'a, E>(
        &self,
        permit: mpsc::Permit<'a, DynEnvelope<A>>,
        envelope: Box<E>,
    ) -> Result<(), RejectedAdmission<'a, A, E>>
    where
        E: Envelope<A> + 'static,
    {
        self.control.transact(move |mode| {
            if mode == Mode::Running {
                permit.send(envelope as DynEnvelope<A>);
                (mode, Ok(()))
            } else {
                (mode, Err((permit, envelope)))
            }
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CallPhase {
    Queued,
    Dispatching,
}

pub(super) struct DispatchPermit {
    control: Arc<Control>,
}

pub(crate) struct HookEntryPermit(());

pub(super) struct CompletionPermit(());

impl DispatchPermit {
    /// Linearizes successful completion with Kill/failure. The unit permit
    /// carries the decision beyond the transaction without exposing lifecycle
    /// state.
    pub(super) fn begin_completion(&self) -> Result<CompletionPermit, CallError> {
        self.control.transact(|mode| {
            let result = match mode {
                Mode::Running | Mode::Draining | Mode::Stopping => Ok(CompletionPermit(())),
                Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_) => {
                    Err(Control::call_failure_for(mode, CallPhase::Dispatching))
                }
            };
            (mode, result)
        })
    }

    /// Commits handler failure before making its dispatch error observable.
    pub(super) fn fail(&self) -> CallError {
        self.control.begin_failure();
        self.control.call_failure(CallPhase::Dispatching)
    }

    /// Borrows the gate retained by this unforgeable dispatch proof.
    pub(super) fn control(&self) -> &Control {
        &self.control
    }
}

#[cfg(test)]
mod tests;
