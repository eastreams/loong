use std::sync::{Arc, Barrier};

use loac::{
    Actor, ActorScope, CallError, ExitReason, Message, RawHandler, ReplyExt, Shutdown,
    ShutdownStatus, StopScope, actor,
};
use tokio::sync::oneshot;

use crate::support::watchdog;

struct PanicActor;

#[actor(mailbox)]
impl Actor for PanicActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(raw = ())]
struct PanicNow;

impl RawHandler<PanicNow> for PanicActor {
    fn handle(
        &mut self,
        _message: PanicNow,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, PanicNow> + use<> {
        panic!("intentional handler panic");
        #[allow(unreachable_code)]
        ().ready()
    }
}

// Handler panics are contained and reported as exit reason.
#[tokio::test]
async fn handler_panics_are_contained_and_reported() {
    let mut owner = loac::spawn::<PanicActor>(());
    let actor = owner.actor_ref();

    assert_eq!(
        watchdog(actor.call(PanicNow)).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Panicked);
    assert_eq!(actor.exit_status().unwrap().reason(), ExitReason::Panicked);
}

#[derive(Message)]
#[message(raw = ())]
struct PanicAfterBarrier {
    entered: oneshot::Sender<()>,
    barrier: Arc<Barrier>,
}

impl RawHandler<PanicAfterBarrier> for PanicActor {
    fn handle(
        &mut self,
        message: PanicAfterBarrier,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, PanicAfterBarrier> + use<> {
        async move {
            let _ = message.entered.send(());
            message.barrier.wait();
            panic!("panic loses to an already committed Kill");
        }
    }
}

// Kill committed during a handler poll wins over panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_committed_during_a_handler_poll_wins_over_panic() {
    let mut owner = loac::spawn::<PanicActor>(());
    let actor = owner.actor_ref();
    let barrier = Arc::new(Barrier::new(2));
    let (entered_tx, entered_rx) = oneshot::channel();
    let response = actor
        .try_call(PanicAfterBarrier {
            entered: entered_tx,
            barrier: barrier.clone(),
        })
        .unwrap();

    watchdog(entered_rx).await.unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    barrier.wait();

    assert_eq!(
        watchdog(response).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Killed);
}

struct KillOnStop;

#[actor]
impl Actor for KillOnStop {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }

    async fn on_stop(&mut self, _reason: ExitReason, scope: &mut StopScope<'_, Self>) {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
    }
}

// Kill requested at graceful finalization wins atomically.
#[tokio::test]
async fn kill_requested_at_graceful_finalization_wins_atomically() {
    let owner = loac::spawn::<KillOnStop>(());

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Killed
    );
}
