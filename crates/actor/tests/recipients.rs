use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use loac::{
    Actor, ActorScope, ExitReason, Handler, Message, Recipient, Shutdown, SyncHandler,
    TryCallErrorKind, TrySendErrorKind, actor,
};
use tokio::sync::oneshot;

async fn watchdog<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .expect("recipient operation exceeded the deadlock watchdog")
}

struct Left;

#[actor(mailbox)]
impl Actor for Left {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Right;

#[actor(mailbox)]
impl Actor for Right {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Query(u64);

impl SyncHandler<Query> for Left {
    fn handle(&mut self, message: Query, _scope: &mut ActorScope<Self>) -> u64 {
        message.0 + 1
    }
}

impl SyncHandler<Query> for Right {
    fn handle(&mut self, message: Query, _scope: &mut ActorScope<Self>) -> u64 {
        message.0 * 2
    }
}

#[derive(Message)]
struct Notify(oneshot::Sender<()>);

impl SyncHandler<Notify> for Left {
    fn handle(&mut self, message: Notify, _scope: &mut ActorScope<Self>) {
        let _ = message.0.send(());
    }
}

impl SyncHandler<Notify> for Right {
    fn handle(&mut self, message: Notify, _scope: &mut ActorScope<Self>) {
        let _ = message.0.send(());
    }
}

fn assert_send_sync<T: Send + Sync>() {}

#[tokio::test]
async fn recipient_erases_actor_type_but_keeps_message_type() {
    let left = loac::spawn::<Left>(());
    let right = loac::spawn::<Right>(());
    let recipients: Vec<Recipient<Query>> = vec![
        left.actor_ref().recipient::<Query>(),
        right.actor_ref().recipient::<Query>(),
    ];
    assert_send_sync::<Recipient<Query>>();
    assert_send_sync::<Recipient<Notify>>();

    assert_eq!(watchdog(recipients[0].call(Query(4))).await.unwrap(), 5);
    assert_eq!(watchdog(recipients[1].call(Query(4))).await.unwrap(), 8);

    let notify_left = left.actor_ref().recipient::<Notify>();
    let notify_right = right.actor_ref().recipient::<Notify>();
    let (left_tx, left_rx) = oneshot::channel();
    let (right_tx, right_rx) = oneshot::channel();
    notify_left.try_send(Notify(left_tx)).unwrap();
    notify_right.send(Notify(right_tx)).await.unwrap();
    watchdog(left_rx).await.unwrap();
    watchdog(right_rx).await.unwrap();

    assert_eq!(
        watchdog(left.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
    assert_eq!(
        watchdog(right.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

struct GateActor {
    seen: Arc<AtomicUsize>,
}

#[actor(mailbox = 1)]
impl Actor for GateActor {
    type SpawnArgs = Arc<AtomicUsize>;

    async fn init(seen: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { seen }
    }
}

#[derive(Message)]
#[message(reply = u8)]
struct SlowQuery {
    value: u8,
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
}

impl SlowQuery {
    fn idle(value: u8) -> Self {
        Self {
            value,
            entered: None,
            release: None,
        }
    }
}

impl Handler<SlowQuery> for GateActor {
    fn handle(
        &mut self,
        message: SlowQuery,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, SlowQuery> + use<> {
        async move {
            if let Some(entered) = message.entered {
                let _ = entered.send(());
            }
            if let Some(release) = message.release {
                let _ = release.await;
            }
            message.value
        }
    }
}

#[derive(Message)]
struct Mark(u8);

impl SyncHandler<Mark> for GateActor {
    fn handle(&mut self, message: Mark, _scope: &mut ActorScope<Self>) {
        self.seen.fetch_add(message.0 as usize, Ordering::SeqCst);
    }
}

#[derive(Message)]
#[message(reply = usize)]
struct Snapshot;

impl SyncHandler<Snapshot> for GateActor {
    fn handle(&mut self, _message: Snapshot, _scope: &mut ActorScope<Self>) -> usize {
        self.seen.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn recipient_preserves_admission_errors_and_one_way_delivery() {
    let seen = Arc::new(AtomicUsize::new(0));
    let mut owner = loac::spawn::<GateActor>(Arc::clone(&seen));
    let actor = owner.actor_ref();
    let queries = actor.recipient::<SlowQuery>();
    let marks = actor.recipient::<Mark>();

    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let queries = queries.clone();
        async move {
            queries
                .call(SlowQuery {
                    value: 1,
                    entered: Some(entered_tx),
                    release: Some(release_rx),
                })
                .await
        }
    });
    watchdog(entered_rx).await.unwrap();

    let queued = queries.try_call(SlowQuery::idle(2)).unwrap();
    let full = queries.try_call(SlowQuery::idle(3)).unwrap_err();
    assert_eq!(full.kind(), TryCallErrorKind::Full);
    assert_eq!(full.into_message().value, 3);
    drop(queued);

    let full_send = marks.try_send(Mark(4)).unwrap_err();
    assert_eq!(full_send.kind(), TrySendErrorKind::Full);
    assert_eq!(full_send.into_message().0, 4);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(1));

    marks.try_send(Mark(5)).unwrap();
    assert_eq!(watchdog(actor.call(Snapshot)).await.unwrap(), 5);
    marks.send(Mark(6)).await.unwrap();
    assert_eq!(watchdog(actor.call(Snapshot)).await.unwrap(), 11);

    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        loac::ShutdownStatus::Requested
    );
    let closed_call = queries.try_call(SlowQuery::idle(7)).unwrap_err();
    assert_eq!(closed_call.kind(), TryCallErrorKind::Closed);
    assert_eq!(closed_call.into_message().value, 7);
    let closed_send = marks.try_send(Mark(8)).unwrap_err();
    assert_eq!(closed_send.kind(), TrySendErrorKind::Closed);
    assert_eq!(closed_send.into_message().0, 8);

    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Stopped);
}
