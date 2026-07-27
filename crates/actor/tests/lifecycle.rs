#[path = "lifecycle/fixtures.rs"]
mod fixtures;
#[path = "lifecycle/graceful.rs"]
mod graceful;
mod support;

use std::{
    future::{Future, poll_fn},
    sync::{Arc, Barrier},
};

use loong_actor::{
    Actor, ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message, ReplyExt,
    Shutdown, ShutdownStatus, spawn,
};
use tokio::sync::oneshot;

use fixtures::{DropSignal, LifecycleActor, LifecycleHarness, Step, actor_with_capacity};
use support::{lock, watchdog};

struct ExitedActor;

impl Actor for ExitedActor {}

#[tokio::test]
async fn shutdown_requests_after_exit_report_the_published_reason() {
    for (initial, expected_reason) in [
        (Shutdown::Stop, ExitReason::Stopped),
        (Shutdown::Drain, ExitReason::Drained),
        (Shutdown::Kill, ExitReason::Killed),
    ] {
        let mut owner = spawn(ExitedActor);
        assert_eq!(owner.request_shutdown(initial), ShutdownStatus::Requested);
        assert_eq!(watchdog(owner.wait()).await, expected_reason);

        for requested in [Shutdown::Stop, Shutdown::Drain, Shutdown::Kill] {
            assert_eq!(
                owner.request_shutdown(requested),
                ShutdownStatus::Exited(expected_reason)
            );
        }
    }
}

struct ControlledStart {
    entered: Option<oneshot::Sender<()>>,
    repolled: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    completed: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Actor for ControlledStart {
    async fn on_start(&mut self, _scope: &mut ActorScope<Self>) {
        let release = self.release.take();
        let mut repolled = self.repolled.take();
        let completed = self.completed.take();
        let _dropped = DropSignal(self.dropped.take());
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = release {
            let mut release = std::pin::pin!(release);
            // The first poll parks on release; a later poll acknowledges that a
            // lifecycle wake reached the still-pending hook.
            let mut first_poll = true;
            let _ = poll_fn(|context| {
                if first_poll {
                    first_poll = false;
                } else if let Some(repolled) = repolled.take() {
                    let _ = repolled.send(());
                }
                release.as_mut().poll(context)
            })
            .await;
        }
        if let Some(completed) = completed {
            let _ = completed.send(());
        }
    }
}

#[tokio::test]
async fn stop_and_drain_wait_for_an_entered_lifecycle_hook() {
    for (shutdown, expected_reason) in [
        (Shutdown::Stop, ExitReason::Stopped),
        (Shutdown::Drain, ExitReason::Drained),
    ] {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (repolled_tx, repolled_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let mut owner = spawn(ControlledStart {
            entered: Some(entered_tx),
            repolled: Some(repolled_tx),
            release: Some(release_rx),
            completed: Some(completed_tx),
            dropped: None,
        });
        let actor = owner.actor_ref();

        watchdog(entered_rx).await.unwrap();
        assert_eq!(owner.request_shutdown(shutdown), ShutdownStatus::Requested);
        watchdog(repolled_rx).await.unwrap();
        assert_eq!(owner.exit_reason(), None);
        assert_eq!(actor.exit_reason(), None);
        assert_eq!(
            owner.request_shutdown(shutdown),
            ShutdownStatus::InProgress(shutdown)
        );

        release_tx.send(()).unwrap();
        watchdog(completed_rx).await.unwrap();
        assert_eq!(watchdog(owner.wait()).await, expected_reason);
        assert_eq!(actor.exit_reason(), Some(expected_reason));
    }
}

#[tokio::test]
async fn kill_cancels_a_pending_lifecycle_hook() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel();
    let (completed_tx, completed_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let mut owner = spawn(ControlledStart {
        entered: Some(entered_tx),
        repolled: None,
        release: Some(release_rx),
        completed: Some(completed_tx),
        dropped: Some(dropped_tx),
    });
    let actor = owner.actor_ref();

    watchdog(entered_rx).await.unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    watchdog(dropped_rx).await.unwrap();
    assert!(watchdog(completed_rx).await.is_err());
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
    assert_eq!(actor.exit_reason(), Some(ExitReason::Killed));
}

struct PanicOnStart;

impl Actor for PanicOnStart {
    async fn on_start(&mut self, _scope: &mut ActorScope<Self>) {
        panic!("intentional lifecycle hook panic");
    }
}

#[tokio::test]
async fn lifecycle_hook_panics_are_contained_and_reported() {
    let mut owner = spawn(PanicOnStart);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(owner.wait()).await, ExitReason::Panicked);
    assert_eq!(actor.exit_reason(), Some(ExitReason::Panicked));
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
