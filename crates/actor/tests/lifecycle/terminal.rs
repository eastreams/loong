use std::{
    future::Future,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use loong_actor::{
    Actor, ActorOwner, ActorScope, CallError, ExitReason, Handler, Message, Shutdown,
    ShutdownStatus, reply, spawn,
};
use tokio::sync::oneshot;

use super::support::watchdog;

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
#[tokio::test(flavor = "current_thread")]
async fn lifecycle_notification_allows_reentrant_shutdown_from_a_safe_waker() {
    let owner = Arc::new(spawn(ExitedActor));
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
    assert_eq!(watchdog(actor.closed()).await, ExitReason::Killed);
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
        reply::ready(())
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
        reply::owned(async move {
            let _ = message.entered.send(());
            message.barrier.wait();
            panic!("panic loses to an already committed Kill");
        })
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
