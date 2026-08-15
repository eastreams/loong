use loac::{
    Actor, ActorRef, ActorScope, ExitReason, Shutdown, ShutdownStatus, SubtreeStatus, actor,
};
use tokio::sync::oneshot;

use super::support::watchdog;

struct LateChild;

#[actor]
impl Actor for LateChild {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct SpawnDuringInit;

struct SpawnDuringInitArgs {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    spawned: oneshot::Sender<ActorRef<LateChild>>,
}

#[actor(children = unbounded)]
impl Actor for SpawnDuringInit {
    type SpawnArgs = SpawnDuringInitArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = args.entered.send(());
        let _ = args.release.await;
        let Ok(child) = scope.spawn_child::<LateChild>(());
        let child = child.into_actor_ref();
        let _ = args.spawned.send(child);
        Self
    }
}

// Shared setup keeps both graceful observations identical.
async fn assert_init_child_joins_graceful_shutdown(shutdown: Shutdown, reason: ExitReason) {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = loac::spawn::<SpawnDuringInit>(SpawnDuringInitArgs {
        entered: entered_tx,
        release: release_rx,
        spawned: spawned_tx,
    });

    watchdog(entered_rx).await.unwrap();
    assert_eq!(owner.request_shutdown(shutdown), ShutdownStatus::Requested);
    release_tx.send(()).unwrap();
    let child = watchdog(spawned_rx).await.unwrap();

    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), reason);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), reason);
}

// Drain retains entered init. Its late child joins cleanup.
#[tokio::test]
async fn drain_includes_children_spawned_during_init() {
    assert_init_child_joins_graceful_shutdown(Shutdown::Drain, ExitReason::Drained).await;
}

// Stop retains entered init. Its late child joins cleanup.
#[tokio::test]
async fn stop_includes_children_spawned_during_init() {
    assert_init_child_joins_graceful_shutdown(Shutdown::Stop, ExitReason::Stopped).await;
}

struct SpawnAfterKill;

#[actor(children = unbounded)]
impl Actor for SpawnAfterKill {
    type SpawnArgs = oneshot::Sender<ActorRef<LateChild>>;

    async fn init(spawned: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
        let Ok(child) = scope.spawn_child::<LateChild>(());
        let child = child.into_actor_ref();
        let _ = spawned.send(child);
        Self
    }
}

// Kill cannot interrupt a poll. Its new child must not escape cleanup.
#[tokio::test]
async fn kill_includes_children_spawned_before_the_current_poll_returns() {
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = loac::spawn::<SpawnAfterKill>(spawned_tx);

    let child = watchdog(spawned_rx).await.unwrap();
    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), ExitReason::Killed);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Killed);
}
