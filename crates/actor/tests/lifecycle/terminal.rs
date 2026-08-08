use std::{
    future::Future,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use loac::{
    Actor, ActorFuture, ActorOwner, ActorScope, CallError, ExitReason, Handler,
    InterleavedFutureExt, Message, ReplyExt, Shutdown, ShutdownStatus, StopScope, SubtreeStatus,
    actor,
};
use tokio::sync::oneshot;

use super::support::watchdog;

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

struct PendingInit;

#[actor(mailbox)]
impl Actor for PendingInit {
    type SpawnArgs = oneshot::Sender<()>;

    async fn init(entered: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = entered.send(());
        std::future::pending::<Self>().await
    }
}

#[derive(Message)]
struct QueuedDrop {
    dropped: Arc<AtomicBool>,
    dropped_while_unwinding: Arc<AtomicBool>,
    panic: bool,
}

impl Drop for QueuedDrop {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
        assert!(!self.panic, "intentional queued message drop panic");
    }
}

impl Handler<QueuedDrop> for PendingInit {
    fn handle(
        &mut self,
        _message: QueuedDrop,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, QueuedDrop> + use<> {
        unreachable!("pending initialization prevents dispatch");
        #[allow(unreachable_code)]
        ().ready()
    }
}

// Executor teardown must use the accepted-envelope cleanup boundary.
// Earlier panics cannot skip later messages or unwind their destructors.
// Synchronous abort also loses subtree confirmation.
#[test]
fn executor_teardown_discards_each_accepted_message() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let call_dropped = Arc::new(AtomicBool::new(false));
    let send_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_unwinding = Arc::new(AtomicBool::new(false));

    let (owner, response) = runtime.block_on(async {
        let owner = loac::spawn::<PendingInit>(entered_tx);
        entered_rx.await.unwrap();
        let actor = owner.actor_ref();
        let response = actor
            .try_call(QueuedDrop {
                dropped: Arc::clone(&call_dropped),
                dropped_while_unwinding: Arc::new(AtomicBool::new(false)),
                panic: true,
            })
            .unwrap();
        actor
            .try_send(QueuedDrop {
                dropped: Arc::clone(&send_dropped),
                dropped_while_unwinding: Arc::new(AtomicBool::new(false)),
                panic: true,
            })
            .unwrap();
        actor
            .try_send(QueuedDrop {
                dropped: Arc::clone(&tail_dropped),
                dropped_while_unwinding: Arc::clone(&tail_unwinding),
                panic: false,
            })
            .unwrap();
        (owner, response)
    });
    let actor = owner.actor_ref();

    drop(runtime);

    assert!(call_dropped.load(Ordering::SeqCst));
    assert!(send_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_unwinding.load(Ordering::SeqCst));
    let mut response = Box::pin(response);
    let mut task = Context::from_waker(Waker::noop());
    assert_eq!(
        response.as_mut().poll(&mut task),
        Poll::Ready(Err(CallError::BeforeDispatch(ExitReason::Aborted)))
    );
    let status = actor
        .exit_status()
        .expect("actor teardown publishes a status");
    assert_eq!(status.reason(), ExitReason::Aborted);
    assert_eq!(status.subtree(), SubtreeStatus::Unconfirmed);
    drop(owner);
}

struct PendingInterleavedActor;

#[actor(mailbox = 2, interleaved = 2)]
impl Actor for PendingInterleavedActor {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
struct PendingInterleavedDrop {
    entered: Option<oneshot::Sender<()>>,
    drops: Arc<AtomicUsize>,
    dropped_while_unwinding: Arc<AtomicBool>,
}

impl ActorFuture<PendingInterleavedActor> for PendingInterleavedDrop {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        _actor: &mut PendingInterleavedActor,
        _scope: &mut ActorScope<'_, PendingInterleavedActor>,
        _task: &mut Context<'_>,
    ) -> Poll<()> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        Poll::Pending
    }
}

impl Drop for PendingInterleavedDrop {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        self.dropped_while_unwinding
            .store(std::thread::panicking(), Ordering::SeqCst);
        panic!("intentional interleaved future drop panic");
    }
}

impl Handler<PendingInterleavedDrop> for PendingInterleavedActor {
    fn handle(
        &mut self,
        message: PendingInterleavedDrop,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, PendingInterleavedDrop> + use<> {
        message.interleaved()
    }
}

// Executor teardown drops scheduler entries without lifecycle access.
// Each entry still needs its own unwind boundary.
#[test]
fn executor_teardown_contains_each_interleaved_drop_panic() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    let (first_entered_tx, first_entered_rx) = oneshot::channel();
    let (second_entered_tx, second_entered_rx) = oneshot::channel();

    let (owner, first, second) = runtime.block_on(async {
        let owner = loac::spawn::<PendingInterleavedActor>(());
        let actor = owner.actor_ref();
        let first = actor
            .try_call(PendingInterleavedDrop {
                entered: Some(first_entered_tx),
                drops: Arc::clone(&drops),
                dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            })
            .unwrap();
        let second = actor
            .try_call(PendingInterleavedDrop {
                entered: Some(second_entered_tx),
                drops: Arc::clone(&drops),
                dropped_while_unwinding: Arc::clone(&dropped_while_unwinding),
            })
            .unwrap();
        first_entered_rx.await.unwrap();
        second_entered_rx.await.unwrap();
        (owner, first, second)
    });
    let actor = owner.actor_ref();

    drop(runtime);

    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert!(!dropped_while_unwinding.load(Ordering::SeqCst));
    for mut response in [Box::pin(first), Box::pin(second)] {
        let mut task = Context::from_waker(Waker::noop());
        assert_eq!(
            response.as_mut().poll(&mut task),
            Poll::Ready(Err(CallError::DuringDispatch(ExitReason::Aborted)))
        );
    }
    let status = actor
        .exit_status()
        .expect("actor teardown publishes a status");
    assert_eq!(status.reason(), ExitReason::Aborted);
    assert_eq!(status.subtree(), SubtreeStatus::Unconfirmed);
    drop(owner);
}

struct PanicActor;

#[actor(mailbox)]
impl Actor for PanicActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
struct PanicNow;

impl Handler<PanicNow> for PanicActor {
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
struct PanicAfterBarrier {
    entered: oneshot::Sender<()>,
    barrier: Arc<Barrier>,
}

impl Handler<PanicAfterBarrier> for PanicActor {
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
