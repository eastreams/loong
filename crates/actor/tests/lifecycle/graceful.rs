use std::num::NonZeroUsize;

use loong_actor::{
    Actor, ActorFutureExt, ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message,
    ReplyExt, Shutdown, ShutdownStatus, SpawnOptions, TryCallErrorKind, spawn_with,
};
use tokio::sync::oneshot;

use super::{
    fixtures::{LifecycleActor, LifecycleHarness, Step, actor_with_capacity},
    support::{lock, watchdog},
};

#[derive(Message)]
struct StopFromExclusive;

impl Handler<StopFromExclusive> for LifecycleActor {
    fn handle(
        &mut self,
        _message: StopFromExclusive,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, StopFromExclusive> + use<> {
        async {}
            .into_actor()
            .map(|(), _actor: &mut Self, scope| {
                assert_eq!(
                    scope.request_shutdown(Shutdown::Stop),
                    ShutdownStatus::Requested
                );
            })
            .exclusive()
    }
}

#[tokio::test]
async fn exclusive_completion_can_commit_graceful_shutdown_without_repoll() {
    let LifecycleHarness {
        mut owner, cleanup, ..
    } = actor_with_capacity(1);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(StopFromExclusive)).await, Ok(()));
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Stopped);
    assert_eq!(*lock(&cleanup), vec![ExitReason::Stopped]);
}

#[tokio::test]
async fn stop_finishes_current_and_cancels_queued_messages() {
    let LifecycleHarness {
        mut owner,
        handled,
        cleanup,
    } = actor_with_capacity(2);
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Step {
                    id: 1,
                    entered: Some(entered_tx),
                    release: Some(release_rx),
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let queued = actor.try_call(Step::immediate(2)).unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        ShutdownStatus::Requested
    );
    assert_eq!(
        actor.try_call(Step::immediate(3)).unwrap_err().kind(),
        TryCallErrorKind::Closed
    );

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(1));
    assert_eq!(
        watchdog(queued).await,
        Err(CallError::BeforeDispatch(ExitReason::Stopped))
    );
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Stopped);
    assert_eq!(*lock(&handled), vec![1]);
    assert_eq!(*lock(&cleanup), vec![ExitReason::Stopped]);
}

#[tokio::test]
async fn drain_runs_the_fixed_accepted_queue_in_order() {
    let LifecycleHarness {
        mut owner,
        handled,
        cleanup,
    } = actor_with_capacity(3);
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Step {
                    id: 1,
                    entered: Some(entered_tx),
                    release: Some(release_rx),
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let second = actor.try_call(Step::immediate(2)).unwrap();
    let third = actor.try_call(Step::immediate(3)).unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Drain),
        ShutdownStatus::Requested
    );
    assert_eq!(
        actor.try_call(Step::immediate(4)).unwrap_err().kind(),
        TryCallErrorKind::Closed
    );

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(1));
    assert_eq!(watchdog(second).await, Ok(2));
    assert_eq!(watchdog(third).await, Ok(3));
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Drained);
    assert_eq!(*lock(&handled), vec![1, 2, 3]);
    assert_eq!(*lock(&cleanup), vec![ExitReason::Drained]);
}

struct InterleavedDrainActor;

impl Actor for InterleavedDrainActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = u8)]
struct InterleavedDrainStep {
    id: u8,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Handler<InterleavedDrainStep> for InterleavedDrainActor {
    fn handle(
        &mut self,
        message: InterleavedDrainStep,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedDrainStep> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            message.id
        }
        .into_actor()
        .interleaved()
    }
}

#[tokio::test]
async fn drain_respects_max_in_flight_for_the_fixed_interleaved_queue() {
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(3).unwrap())
        .with_max_in_flight(NonZeroUsize::new(2).unwrap());
    let mut owner = spawn_with::<InterleavedDrainActor>((), options);
    let actor = owner.actor_ref();

    let (first_entered_tx, first_entered_rx) = oneshot::channel();
    let (first_release_tx, first_release_rx) = oneshot::channel();
    let first = actor
        .try_call(InterleavedDrainStep {
            id: 1,
            entered: first_entered_tx,
            release: first_release_rx,
        })
        .unwrap();
    let (second_entered_tx, second_entered_rx) = oneshot::channel();
    let (second_release_tx, second_release_rx) = oneshot::channel();
    let second = actor
        .try_call(InterleavedDrainStep {
            id: 2,
            entered: second_entered_tx,
            release: second_release_rx,
        })
        .unwrap();
    watchdog(first_entered_rx).await.unwrap();
    watchdog(second_entered_rx).await.unwrap();

    let (third_entered_tx, mut third_entered_rx) = oneshot::channel();
    let (third_release_tx, third_release_rx) = oneshot::channel();
    let third = actor
        .try_call(InterleavedDrainStep {
            id: 3,
            entered: third_entered_tx,
            release: third_release_rx,
        })
        .unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Drain),
        ShutdownStatus::Requested
    );
    let (fourth_entered_tx, _fourth_entered_rx) = oneshot::channel();
    let (_fourth_release_tx, fourth_release_rx) = oneshot::channel();
    assert_eq!(
        actor
            .try_call(InterleavedDrainStep {
                id: 4,
                entered: fourth_entered_tx,
                release: fourth_release_rx,
            })
            .unwrap_err()
            .kind(),
        TryCallErrorKind::Closed
    );
    assert!(matches!(
        third_entered_rx.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));

    first_release_tx.send(()).unwrap();
    watchdog(third_entered_rx).await.unwrap();
    second_release_tx.send(()).unwrap();
    third_release_tx.send(()).unwrap();

    assert_eq!(watchdog(first).await, Ok(1));
    assert_eq!(watchdog(second).await, Ok(2));
    assert_eq!(watchdog(third).await, Ok(3));
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Drained);
}
