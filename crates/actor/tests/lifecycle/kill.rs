use std::{future, sync::mpsc as std_mpsc, time::Duration};

use loong_actor::{
    ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message, ReplyExt, Shutdown,
    ShutdownStatus,
};
use tokio::sync::oneshot;

use super::{
    fixtures::{DropSignal, LifecycleActor, LifecycleHarness, Step, actor_with_capacity},
    support::{lock, watchdog},
};

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

struct OwnedInterruptible {
    entered: oneshot::Sender<()>,
    drop_barrier: DropBarrier,
}

struct DropBarrier {
    entered: Option<oneshot::Sender<()>>,
    release: std_mpsc::Receiver<()>,
}

impl Drop for DropBarrier {
    fn drop(&mut self) {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        let _ = self.release.recv();
    }
}

impl Message for OwnedInterruptible {
    type Reply = ();
}

impl Handler<OwnedInterruptible> for LifecycleActor {
    fn handle(
        &mut self,
        message: OwnedInterruptible,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, OwnedInterruptible> + use<> {
        async move {
            let _drop_barrier = message.drop_barrier;
            let _ = message.entered.send(());
            future::pending().await
        }
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

// The destructor blocks after confirming cancellation.
// Killed must remain unpublished until that barrier opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_joins_owned_reply_cancellation_before_publishing_exit() {
    let mut owner = actor_with_capacity(1).owner;
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (drop_entered_tx, drop_entered_rx) = oneshot::channel();
    let (drop_release_tx, drop_release_rx) = std_mpsc::channel();
    let response = actor
        .try_call(OwnedInterruptible {
            entered: entered_tx,
            drop_barrier: DropBarrier {
                entered: Some(drop_entered_tx),
                release: drop_release_rx,
            },
        })
        .unwrap();
    watchdog(entered_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    let wait = owner.wait();
    tokio::pin!(wait);
    watchdog(drop_entered_rx).await.unwrap();
    let early = tokio::time::timeout(Duration::from_millis(20), wait.as_mut()).await;
    drop_release_tx.send(()).unwrap();

    assert!(early.is_err());
    assert_eq!(watchdog(wait).await, ExitReason::Killed);
    assert_eq!(
        watchdog(response).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
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
