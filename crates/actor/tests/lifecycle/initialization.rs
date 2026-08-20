use std::{
    future::{Future, poll_fn},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
};

use loac::{
    Actor, ActorScope, CallError, ExitReason, Handler, Message, ReplyExt, Shutdown, ShutdownStatus,
    StopScope, TrySendErrorKind, actor,
};
use tokio::sync::oneshot;

use super::support::{lock, watchdog};

struct AdmissionArgs {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    constructed: Arc<AtomicUsize>,
    handled: Arc<AtomicUsize>,
}

struct AdmissionActor {
    value: u8,
    handled: Arc<AtomicUsize>,
}

#[actor(mailbox = 2)]
impl Actor for AdmissionActor {
    type SpawnArgs = AdmissionArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = args.entered.send(());
        let _ = args.release.await;
        args.constructed.fetch_add(1, Ordering::SeqCst);
        Self {
            value: 40,
            handled: args.handled,
        }
    }
}

#[derive(Message)]
struct Notify(u8);

impl Handler<Notify> for AdmissionActor {
    fn handle(
        &mut self,
        message: Notify,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Notify> + use<> {
        self.value += message.0;
        self.handled.fetch_add(1, Ordering::SeqCst);
        ().ready()
    }
}

#[derive(Message)]
#[message(reply = (u8, usize))]
struct Read;

impl Handler<Read> for AdmissionActor {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Read> + use<> {
        (self.value, self.handled.load(Ordering::SeqCst)).ready()
    }
}

// `Full` proves both earlier messages reached the mailbox.
// Zero counters prove neither construction nor dispatch ran early.
// Messages are admitted but not dispatched before init is ready.
#[tokio::test]
async fn messages_are_admitted_but_not_dispatched_before_init_ready() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let constructed = Arc::new(AtomicUsize::new(0));
    let handled = Arc::new(AtomicUsize::new(0));
    let owner = loac::spawn::<AdmissionActor>(AdmissionArgs {
        entered: entered_tx,
        release: release_rx,
        constructed: Arc::clone(&constructed),
        handled: Arc::clone(&handled),
    });
    let actor = owner.actor_ref();

    watchdog(entered_rx).await.unwrap();
    watchdog(actor.send(Notify(2))).await.unwrap();
    let mut response = Box::pin(actor.call(Read));
    let first_poll = poll_fn(|task| Poll::Ready(response.as_mut().poll(task))).await;
    assert!(first_poll.is_pending());
    assert_eq!(
        actor.try_send(Notify(9)).unwrap_err().kind(),
        TrySendErrorKind::Full
    );
    assert_eq!(constructed.load(Ordering::SeqCst), 0);
    assert_eq!(handled.load(Ordering::SeqCst), 0);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(response).await, Ok((42, 1)));
    assert_eq!(constructed.load(Ordering::SeqCst), 1);
    assert_eq!(handled.load(Ordering::SeqCst), 1);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}

struct ControlledInitArgs {
    entered: oneshot::Sender<()>,
    repolled: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    completed: oneshot::Sender<()>,
    handled: Arc<AtomicUsize>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

struct ControlledInit {
    handled: Arc<AtomicUsize>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

#[actor(mailbox)]
impl Actor for ControlledInit {
    type SpawnArgs = ControlledInitArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let ControlledInitArgs {
            entered,
            repolled,
            release,
            completed,
            handled,
            cleanup,
        } = args;
        let _ = entered.send(());
        let mut release = std::pin::pin!(release);
        let mut repolled = Some(repolled);
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
        let actor = Self { handled, cleanup };
        let _ = completed.send(());
        actor
    }

    async fn on_stop(&mut self, reason: ExitReason, _scope: &mut StopScope<'_, Self>) {
        lock(&self.cleanup).push(reason);
    }
}

#[derive(Message)]
#[message(reply = ())]
struct InitPing;

impl Handler<InitPing> for ControlledInit {
    fn handle(
        &mut self,
        _message: InitPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, InitPing> + use<> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        ().ready()
    }
}

// Graceful modes must retain initialization.
// The response distinguishes Stop discard from Drain dispatch.
// Stop and Drain wait for init, then apply their queue policies.
#[tokio::test]
async fn stop_and_drain_wait_for_init_then_apply_queue_policy() {
    for (shutdown, expected_reason) in [
        (Shutdown::Stop, ExitReason::Stopped),
        (Shutdown::Drain, ExitReason::Drained),
    ] {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (repolled_tx, repolled_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let handled = Arc::new(AtomicUsize::new(0));
        let cleanup = Arc::new(Mutex::new(Vec::new()));
        let mut owner = loac::spawn::<ControlledInit>(ControlledInitArgs {
            entered: entered_tx,
            repolled: repolled_tx,
            release: release_rx,
            completed: completed_tx,
            handled: Arc::clone(&handled),
            cleanup: Arc::clone(&cleanup),
        });
        let actor = owner.actor_ref();

        watchdog(entered_rx).await.unwrap();
        let mut response = Box::pin(actor.try_call(InitPing).unwrap());
        assert_eq!(owner.request_shutdown(shutdown), ShutdownStatus::Requested);
        // Observe the lifecycle wake before releasing init.
        watchdog(repolled_rx).await.unwrap();
        let first_poll = poll_fn(|task| Poll::Ready(response.as_mut().poll(task))).await;
        assert!(first_poll.is_pending());
        assert_eq!(owner.exit_status(), None);
        assert_eq!(actor.exit_status(), None);

        release_tx.send(()).unwrap();
        watchdog(completed_rx).await.unwrap();
        let result = watchdog(response).await;
        match shutdown {
            Shutdown::Stop => {
                assert_eq!(result, Err(CallError::BeforeDispatch(ExitReason::Stopped)));
                assert_eq!(handled.load(Ordering::SeqCst), 0);
            }
            Shutdown::Drain => {
                assert_eq!(result, Ok(()));
                assert_eq!(handled.load(Ordering::SeqCst), 1);
            }
            _ => unreachable!("the test covers graceful modes"),
        }
        assert_eq!(watchdog(owner.wait()).await.reason(), expected_reason);
        assert_eq!(*lock(&cleanup), vec![expected_reason]);
    }
}
