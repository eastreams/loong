use super::*;

use tokio_util::sync::CancellationToken;

struct CancellationActor;

#[actor(mailbox = 1, interleaved = 1)]
impl Actor for CancellationActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Debug, Eq, PartialEq)]
struct WorkCancelled;

#[derive(Message)]
#[message(reply = WorkCancelled)]
struct CancellableWork {
    started: oneshot::Sender<()>,
    cancellation: CancellationToken,
}

impl DispatchHandler<CancellableWork> for CancellationActor {
    fn handle(
        &mut self,
        message: CancellableWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, CancellableWork> + use<> {
        async move {
            let _ = message.started.send(());
            message.cancellation.cancelled().await;
            WorkCancelled
        }
        .into_actor()
        .interleaved()
    }
}

#[derive(Message)]
#[message(reply = ())]
struct CancelWork {
    dispatched: oneshot::Sender<()>,
}

impl DispatchHandler<CancelWork> for CancellationActor {
    fn handle(
        &mut self,
        message: CancelWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, CancelWork> + use<> {
        let _ = message.dispatched.send(());
        ().ready()
    }
}

// A regular mailbox cancellation message cannot reach an active interleaved
// reply while its only `max_in_flight` slot is occupied. A caller-owned token
// wakes that reply directly; only then can the queued message dispatch.
#[tokio::test]
async fn caller_cancellation_bypasses_a_full_interleaved_lane() {
    let owner = loac::spawn::<CancellationActor>(());
    let actor = owner.actor_ref();
    let cancellation = CancellationToken::new();
    let (started_tx, started_rx) = oneshot::channel();
    let active = actor
        .try_call(CancellableWork {
            started: started_tx,
            cancellation: cancellation.clone(),
        })
        .unwrap();

    watchdog(started_rx).await.unwrap();

    let (dispatched_tx, dispatched_rx) = oneshot::channel();
    let mut queued = Box::pin(
        actor
            .try_call(CancelWork {
                dispatched: dispatched_tx,
            })
            .unwrap(),
    );
    assert!(poll_once(queued.as_mut()).await.is_pending());

    cancellation.cancel();
    assert_eq!(watchdog(active).await, Ok(WorkCancelled));
    assert_eq!(watchdog(queued).await, Ok(()));
    watchdog(dispatched_rx).await.unwrap();

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
