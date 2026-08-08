use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use loac::{
    Actor, ActorOwner, ActorScope, ExitReason, Shutdown, ShutdownStatus, SubtreeStatus, actor,
};
use tokio::sync::oneshot;

use crate::support::watchdog;

struct ExitedActor;

#[actor]
impl Actor for ExitedActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

// Shutdown requests after exit report the published status.
#[tokio::test]
async fn shutdown_requests_after_exit_report_the_published_status() {
    for (initial, expected_reason) in [
        (Shutdown::Stop, ExitReason::Stopped),
        (Shutdown::Drain, ExitReason::Drained),
        (Shutdown::Kill, ExitReason::Killed),
    ] {
        let mut owner = loac::spawn::<ExitedActor>(());
        assert_eq!(owner.request_shutdown(initial), ShutdownStatus::Requested);
        let status = watchdog(owner.wait()).await;
        assert_eq!(status.reason(), expected_reason);
        assert_eq!(status.subtree(), SubtreeStatus::Terminated);

        for requested in [Shutdown::Stop, Shutdown::Drain, Shutdown::Kill] {
            assert_eq!(
                owner.request_shutdown(requested),
                ShutdownStatus::Exited(status)
            );
        }
    }
}

struct StateDrop(Option<oneshot::Sender<()>>);

struct StateDropArgs {
    initialized: oneshot::Sender<()>,
    dropped: oneshot::Sender<()>,
}

#[actor]
impl Actor for StateDrop {
    type SpawnArgs = StateDropArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = args.initialized.send(());
        Self(Some(args.dropped))
    }
}

impl Drop for StateDrop {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}

// Actor refs do not keep an actor alive.
#[tokio::test]
async fn actor_refs_do_not_keep_an_actor_alive() {
    let (initialized_tx, initialized_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let owner = loac::spawn::<StateDrop>(StateDropArgs {
        initialized: initialized_tx,
        dropped: dropped_tx,
    });
    let actor = owner.actor_ref();
    let another_ref = actor.clone();

    watchdog(initialized_rx).await.unwrap();
    drop(owner);

    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Killed);
    watchdog(dropped_rx).await.unwrap();
    assert_eq!(
        another_ref.exit_status().unwrap().reason(),
        ExitReason::Killed
    );
}

struct ReentrantShutdownWaker {
    owner: Arc<ActorOwner<ExitedActor>>,
    entered: AtomicBool,
    result: mpsc::SyncSender<ShutdownStatus>,
}

impl ReentrantShutdownWaker {
    fn reenter(&self) {
        if !self.entered.swap(true, Ordering::SeqCst) {
            let _ = self
                .result
                .send(self.owner.request_shutdown(Shutdown::Kill));
        }
    }
}

impl Wake for ReentrantShutdownWaker {
    fn wake(self: Arc<Self>) {
        self.reenter();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.reenter();
    }
}

// Lifecycle observers may use any safe Waker. Requesting Stop on an OS thread
// makes synchronous reentry observable without allowing the old self-deadlock
// to freeze this current-thread runtime or hide behind an async timeout.
// Lifecycle notification allows reentrant shutdown from a safe waker.
#[tokio::test(flavor = "current_thread")]
async fn lifecycle_notification_allows_reentrant_shutdown_from_a_safe_waker() {
    let owner = Arc::new(loac::spawn::<ExitedActor>(()));
    let actor = owner.actor_ref();
    let (kill_tx, kill_rx) = mpsc::sync_channel(1);
    let probe = Arc::new(ReentrantShutdownWaker {
        owner: Arc::clone(&owner),
        entered: AtomicBool::new(false),
        result: kill_tx,
    });
    let waker = Waker::from(Arc::clone(&probe));
    let mut task = Context::from_waker(&waker);
    let mut closed = Box::pin(actor.closed());
    assert_eq!(closed.as_mut().poll(&mut task), Poll::Pending);

    let (stop_tx, stop_rx) = mpsc::sync_channel(1);
    let requester = std::thread::spawn({
        let owner = Arc::clone(&owner);
        move || {
            let _ = stop_tx.send(owner.request_shutdown(Shutdown::Stop));
        }
    });

    assert_eq!(
        kill_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the lifecycle waker must reenter without deadlocking"),
        ShutdownStatus::Requested
    );
    assert_eq!(
        stop_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the outer shutdown request must return after notification"),
        ShutdownStatus::Requested
    );
    requester.join().expect("shutdown thread must not panic");

    drop(closed);
    drop(waker);
    drop(probe);
    drop(owner);
    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Killed);
}
