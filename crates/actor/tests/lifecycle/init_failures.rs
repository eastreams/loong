use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use loong_actor::{
    Actor, ActorScope, CallError, ExitReason, Handler, Message, ReplyExt, Shutdown, ShutdownStatus,
    StopScope, SubtreeStatus, TryCallErrorKind, spawn,
};
use tokio::sync::oneshot;

use super::{fixtures::DropSignal, support::watchdog};

#[derive(Message)]
struct InitPing;

struct NeverReadyArgs {
    entered: oneshot::Sender<()>,
    cancelled: oneshot::Sender<()>,
    actor_drops: Arc<AtomicUsize>,
}

struct NeverReadyInit {
    actor_drops: Arc<AtomicUsize>,
}

impl Actor for NeverReadyInit {
    type SpawnArgs = NeverReadyArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _cancelled = DropSignal(Some(args.cancelled));
        let _ = args.entered.send(());
        std::future::pending::<()>().await;
        Self {
            actor_drops: args.actor_drops,
        }
    }
}

impl Drop for NeverReadyInit {
    fn drop(&mut self) {
        self.actor_drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Handler<InitPing> for NeverReadyInit {
    fn handle(
        &mut self,
        _message: InitPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, InitPing> + use<> {
        ().ready()
    }
}

struct PreKilledArgs {
    constructor_calls: Arc<AtomicUsize>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Drop for PreKilledArgs {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
        panic!("intentional skipped init args drop panic");
    }
}

struct PreKilledActor;

impl Actor for PreKilledActor {
    type SpawnArgs = PreKilledArgs;

    fn init<'a>(
        args: Self::SpawnArgs,
        _scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = Self> + Send + 'a {
        args.constructor_calls.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Self)
    }
}

// Kill commits before the actor task's first poll.
// The counter rejects any later constructor entry.
// A `SpawnArgs` Drop panic must remain contained.
#[tokio::test(flavor = "current_thread")]
async fn preinit_kill_skips_sync_constructor_and_contains_args_drop() {
    let constructor_calls = Arc::new(AtomicUsize::new(0));
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let mut owner = spawn::<PreKilledActor>(PreKilledArgs {
        constructor_calls: Arc::clone(&constructor_calls),
        dropped: Some(dropped_tx),
    });

    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    watchdog(dropped_rx).await.unwrap();
    assert_eq!(constructor_calls.load(Ordering::SeqCst), 0);
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Killed);
}

// Kill must not construct `Self` from a pending init.
// The accepted call must retain its before-dispatch phase.
#[tokio::test]
async fn kill_cancels_never_ready_init_before_actor_construction() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (cancelled_tx, cancelled_rx) = oneshot::channel();
    let actor_drops = Arc::new(AtomicUsize::new(0));
    let mut owner = spawn::<NeverReadyInit>(NeverReadyArgs {
        entered: entered_tx,
        cancelled: cancelled_tx,
        actor_drops: Arc::clone(&actor_drops),
    });
    let actor = owner.actor_ref();

    watchdog(entered_rx).await.unwrap();
    let mut response = Box::pin(actor.try_call(InitPing).unwrap());
    let first_poll = poll_fn(|task| Poll::Ready(response.as_mut().poll(task))).await;
    assert!(first_poll.is_pending());
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    watchdog(cancelled_rx).await.unwrap();
    assert_eq!(
        watchdog(response).await,
        Err(CallError::BeforeDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Killed);
    assert_eq!(actor_drops.load(Ordering::SeqCst), 0);
    assert_eq!(actor.exit_status().unwrap().reason(), ExitReason::Killed);
}

struct InitChild;

impl Actor for InitChild {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl Handler<InitPing> for InitChild {
    fn handle(
        &mut self,
        _message: InitPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, InitPing> + use<> {
        ().ready()
    }
}

struct ReadyDropArgs {
    child_started: oneshot::Sender<loong_actor::ActorRef<InitChild>>,
    actor_drop_observed_kill: oneshot::Sender<bool>,
}

struct ReadyDropActor {
    child: loong_actor::ActorRef<InitChild>,
    actor_drop_observed_kill: Option<oneshot::Sender<bool>>,
}

impl Drop for ReadyDropActor {
    fn drop(&mut self) {
        // This fixture submits no other child shutdown.
        // Closed therefore proves Kill preceded actor Drop.
        let child_is_killing = match self.child.try_call(InitPing) {
            Err(error) => error.kind() == TryCallErrorKind::Closed,
            Ok(response) => {
                drop(response);
                false
            }
        };
        if let Some(observed) = self.actor_drop_observed_kill.take() {
            let _ = observed.send(child_is_killing);
        }
    }
}

struct ReadyThenDropPanic {
    actor: Option<ReadyDropActor>,
}

impl Future for ReadyThenDropPanic {
    type Output = ReadyDropActor;

    fn poll(mut self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.actor.take().expect("init future is polled once"))
    }
}

impl Drop for ReadyThenDropPanic {
    fn drop(&mut self) {
        panic!("intentional ready init future drop panic");
    }
}

impl Actor for ReadyDropActor {
    type SpawnArgs = ReadyDropArgs;

    fn init<'a>(
        args: Self::SpawnArgs,
        scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = Self> + Send + 'a {
        let child = scope.spawn_child::<InitChild>(()).into_actor_ref();
        let _ = args.child_started.send(child.clone());
        ReadyThenDropPanic {
            actor: Some(Self {
                child,
                actor_drop_observed_kill: Some(args.actor_drop_observed_kill),
            }),
        }
    }
}

// Ready actor state remains runtime-owned after future cleanup fails.
// Child cutoff must precede actor state Drop.
#[tokio::test(flavor = "current_thread")]
async fn ready_init_future_drop_panic_kills_child_before_actor_drop() {
    let (child_tx, child_rx) = oneshot::channel();
    let (observed_tx, observed_rx) = oneshot::channel();
    let mut owner = spawn::<ReadyDropActor>(ReadyDropArgs {
        child_started: child_tx,
        actor_drop_observed_kill: observed_tx,
    });

    let child = watchdog(child_rx).await.unwrap();
    assert!(watchdog(observed_rx).await.unwrap());
    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), ExitReason::Panicked);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Killed);
}

struct PanicInitArgs {
    child_started: oneshot::Sender<loong_actor::ActorRef<InitChild>>,
    cleanup: Arc<AtomicUsize>,
    panic_during_call: bool,
}

struct PanicInit {
    cleanup: Arc<AtomicUsize>,
}

impl Actor for PanicInit {
    type SpawnArgs = PanicInitArgs;

    fn init<'a>(
        args: Self::SpawnArgs,
        scope: &'a mut ActorScope<'_, Self>,
    ) -> impl Future<Output = Self> + Send + 'a {
        let child = scope.spawn_child::<InitChild>(()).into_actor_ref();
        let _ = args.child_started.send(child);
        assert!(!args.panic_during_call, "intentional init call panic");
        std::future::ready(Self {
            cleanup: args.cleanup,
        })
    }

    async fn on_stop(&mut self, _reason: ExitReason, _scope: &mut StopScope<'_, Self>) {
        self.cleanup.fetch_add(1, Ordering::SeqCst);
    }
}

impl Handler<InitPing> for PanicInit {
    fn handle(
        &mut self,
        _message: InitPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, InitPing> + use<> {
        ().ready()
    }
}

// A constructor panic happens before any init future exists.
// Registered children and queued calls still require failure teardown.
#[tokio::test(flavor = "current_thread")]
async fn init_call_panic_discards_calls_and_kills_registered_children() {
    let (child_tx, child_rx) = oneshot::channel();
    let cleanup = Arc::new(AtomicUsize::new(0));
    let mut owner = spawn::<PanicInit>(PanicInitArgs {
        child_started: child_tx,
        cleanup: Arc::clone(&cleanup),
        panic_during_call: true,
    });
    let actor = owner.actor_ref();

    // No await may let init run before admission commits.
    let response = actor.try_call(InitPing).unwrap();
    let child = watchdog(child_rx).await.unwrap();

    assert_eq!(
        watchdog(response).await,
        Err(CallError::BeforeDispatch(ExitReason::Panicked))
    );
    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), ExitReason::Panicked);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Killed);
    assert_eq!(cleanup.load(Ordering::SeqCst), 0);
}
