use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use loac::{
    Actor, ActorFuture, ActorScope, CallError, ExitReason, Handler, InterleavedFutureExt, Message,
    ReplyExt, SubtreeStatus, actor,
};
use tokio::sync::oneshot;

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
