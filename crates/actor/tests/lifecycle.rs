mod support;

use std::{
    num::NonZeroUsize,
    sync::{Arc, Barrier, Mutex},
};

use loong_actor::{
    Actor, ActorFutureExt, ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message,
    ReplyExt, Shutdown, ShutdownStatus, SpawnOptions, TryCallErrorKind, spawn, spawn_with,
};
use tokio::sync::oneshot;

use support::{lock, watchdog};

struct LifecycleActor {
    handled: Arc<Mutex<Vec<u8>>>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

impl Actor for LifecycleActor {
    async fn on_stop(&mut self, reason: ExitReason, _scope: &mut ActorScope<Self>) {
        lock(&self.cleanup).push(reason);
    }
}

struct Step {
    id: u8,
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
}

impl Step {
    fn immediate(id: u8) -> Self {
        Self {
            id,
            entered: None,
            release: None,
        }
    }
}

impl Message for Step {
    type Reply = u8;
}

impl Handler<Step> for LifecycleActor {
    fn handle(
        &mut self,
        mut message: Step,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Step> + use<> {
        async move {
            if let Some(entered) = message.entered.take() {
                let _ = entered.send(());
            }
            if let Some(release) = message.release.take() {
                let _ = release.await;
            }
            message.id
        }
        .into_actor()
        .map(|id, actor: &mut Self, _scope| {
            lock(&actor.handled).push(id);
            id
        })
        .exclusive()
    }
}

struct LifecycleHarness {
    owner: loong_actor::ActorOwner<LifecycleActor>,
    handled: Arc<Mutex<Vec<u8>>>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

fn actor_with_capacity(capacity: usize) -> LifecycleHarness {
    let handled = Arc::new(Mutex::new(Vec::new()));
    let cleanup = Arc::new(Mutex::new(Vec::new()));
    let actor = LifecycleActor {
        handled: handled.clone(),
        cleanup: cleanup.clone(),
    };
    let owner = spawn_with(
        actor,
        SpawnOptions::default()
            .with_mailbox_capacity(NonZeroUsize::new(capacity).expect("test capacity is non-zero"))
            .with_max_in_flight(NonZeroUsize::new(1).expect("one is non-zero")),
    );
    LifecycleHarness {
        owner,
        handled,
        cleanup,
    }
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
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Stopped);
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
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Drained);
    assert_eq!(*lock(&handled), vec![1, 2, 3]);
    assert_eq!(*lock(&cleanup), vec![ExitReason::Drained]);
}

struct OwnedDrainActor;

impl Actor for OwnedDrainActor {}

struct OwnedDrainStep {
    id: u8,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for OwnedDrainStep {
    type Reply = u8;
}

impl Handler<OwnedDrainStep> for OwnedDrainActor {
    fn handle(
        &mut self,
        message: OwnedDrainStep,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, OwnedDrainStep> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            message.id
        }
    }
}

#[tokio::test]
async fn drain_respects_max_in_flight_while_finishing_the_fixed_owned_queue() {
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(3).unwrap())
        .with_max_in_flight(NonZeroUsize::new(2).unwrap());
    let mut owner = spawn_with(OwnedDrainActor, options);
    let actor = owner.actor_ref();

    let (first_entered_tx, first_entered_rx) = oneshot::channel();
    let (first_release_tx, first_release_rx) = oneshot::channel();
    let first = actor
        .try_call(OwnedDrainStep {
            id: 1,
            entered: first_entered_tx,
            release: first_release_rx,
        })
        .unwrap();
    let (second_entered_tx, second_entered_rx) = oneshot::channel();
    let (second_release_tx, second_release_rx) = oneshot::channel();
    let second = actor
        .try_call(OwnedDrainStep {
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
        .try_call(OwnedDrainStep {
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
            .try_call(OwnedDrainStep {
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
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Drained);
}

struct DropSignal(Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

struct Interruptible {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    dropped: DropSignal,
}

impl Message for Interruptible {
    type Reply = ();
}

impl Handler<Interruptible> for LifecycleActor {
    fn handle(
        &mut self,
        message: Interruptible,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Interruptible> + use<> {
        async move {
            let _dropped = message.dropped;
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

struct KillBeforeReady;

impl Message for KillBeforeReady {
    type Reply = ();
}

impl Handler<KillBeforeReady> for LifecycleActor {
    fn handle(
        &mut self,
        _message: KillBeforeReady,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, KillBeforeReady> + use<> {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
        ().ready()
    }
}

#[tokio::test]
async fn kill_before_ready_completion_reports_the_dispatching_phase() {
    let mut owner = actor_with_capacity(1).owner;
    let actor = owner.actor_ref();

    assert_eq!(
        watchdog(actor.call(KillBeforeReady)).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
}

#[tokio::test]
async fn kill_drops_current_and_queued_work_without_cleanup() {
    let LifecycleHarness {
        mut owner,
        handled,
        cleanup,
    } = actor_with_capacity(2);
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let current = actor
        .try_call(Interruptible {
            entered: entered_tx,
            release: release_rx,
            dropped: DropSignal(Some(dropped_tx)),
        })
        .unwrap();

    watchdog(entered_rx).await.unwrap();
    let queued = actor.try_call(Step::immediate(2)).unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    watchdog(dropped_rx).await.unwrap();
    assert_eq!(
        watchdog(current).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(
        watchdog(queued).await,
        Err(CallError::BeforeDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
    assert!(lock(&handled).is_empty());
    assert!(lock(&cleanup).is_empty());
}

#[tokio::test]
async fn graceful_mode_is_first_wins_and_kill_can_upgrade_it() {
    let LifecycleHarness {
        mut owner,
        handled: _,
        cleanup,
    } = actor_with_capacity(1);
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let current = actor
        .try_call(Interruptible {
            entered: entered_tx,
            release: release_rx,
            dropped: DropSignal(Some(dropped_tx)),
        })
        .unwrap();
    watchdog(entered_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Drain),
        ShutdownStatus::Requested
    );
    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        ShutdownStatus::InProgress(Shutdown::Drain)
    );
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    watchdog(dropped_rx).await.unwrap();
    assert_eq!(
        watchdog(current).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
    assert!(lock(&cleanup).is_empty());
}

struct StateDrop(Option<oneshot::Sender<()>>);

impl Actor for StateDrop {}

impl Drop for StateDrop {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

#[tokio::test]
async fn actor_refs_do_not_keep_an_actor_alive() {
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let owner = spawn(StateDrop(Some(dropped_tx)));
    let actor = owner.actor_ref();
    let another_ref = actor.clone();

    drop(owner);

    assert_eq!(watchdog(actor.closed()).await, ExitReason::Killed);
    watchdog(dropped_rx).await.unwrap();
    assert_eq!(another_ref.exit_reason(), Some(ExitReason::Killed));
}

struct PendingStart {
    entered: Option<oneshot::Sender<()>>,
}

impl Actor for PendingStart {
    async fn on_start(&mut self, _scope: &mut ActorScope<Self>) {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        std::future::pending().await
    }
}

#[test]
fn executor_teardown_after_kill_reports_the_weaker_aborted_reason() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let owner = runtime.block_on(async {
        let owner = spawn(PendingStart {
            entered: Some(entered_tx),
        });
        entered_rx.await.unwrap();
        owner
    });
    let actor = owner.actor_ref();

    drop(owner);
    drop(runtime);

    assert_eq!(actor.exit_reason(), Some(ExitReason::Aborted));
}

struct PanicActor;

impl Actor for PanicActor {}

struct PanicNow;

impl Message for PanicNow {
    type Reply = ();
}

impl Handler<PanicNow> for PanicActor {
    fn handle(
        &mut self,
        _message: PanicNow,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PanicNow> + use<> {
        panic!("intentional handler panic");
        #[allow(unreachable_code)]
        ().ready()
    }
}

#[tokio::test]
async fn handler_panics_are_contained_and_reported() {
    let mut owner = spawn(PanicActor);
    let actor = owner.actor_ref();

    assert_eq!(
        watchdog(actor.call(PanicNow)).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Panicked);
    assert_eq!(actor.exit_reason(), Some(ExitReason::Panicked));
}

struct PanicAfterBarrier {
    entered: oneshot::Sender<()>,
    barrier: Arc<Barrier>,
}

impl Message for PanicAfterBarrier {
    type Reply = ();
}

impl Handler<PanicAfterBarrier> for PanicActor {
    fn handle(
        &mut self,
        message: PanicAfterBarrier,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PanicAfterBarrier> + use<> {
        async move {
            let _ = message.entered.send(());
            message.barrier.wait();
            panic!("panic loses to an already committed Kill");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_committed_during_a_handler_poll_wins_over_panic() {
    let mut owner = spawn(PanicActor);
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
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
}

struct KillOnStop;

impl Actor for KillOnStop {
    async fn on_stop(&mut self, _reason: ExitReason, scope: &mut ActorScope<Self>) {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
    }
}

#[tokio::test]
async fn kill_requested_at_graceful_finalization_wins_atomically() {
    let owner = spawn(KillOnStop);

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Killed
    );
}
