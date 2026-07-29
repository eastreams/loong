mod support;

use std::{
    future::Future,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    task::Poll,
};

use loong_actor::{
    Actor, ActorFutureExt, ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message,
    ReplyExt, Shutdown, SpawnOptions, SyncHandler, TryCallErrorKind, TrySendErrorKind, spawn,
    spawn_with,
};
use tokio::sync::{mpsc, oneshot};

use support::{lock, watchdog};

struct Calculator(u64);

impl Actor for Calculator {}

struct Add(u64);

impl Message for Add {
    type Reply = u64;
}

impl Handler<Add> for Calculator {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Add> + use<> {
        self.0 += message.0;
        self.0.ready()
    }
}

struct Describe;

impl Message for Describe {
    type Reply = String;
}

impl Handler<Describe> for Calculator {
    fn handle(
        &mut self,
        _message: Describe,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Describe> + use<> {
        format!("count={}", self.0).ready()
    }
}

#[tokio::test]
async fn one_actor_handles_multiple_typed_message_replies() {
    let owner = spawn(Calculator(0));
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(Add(3))).await.unwrap(), 3);
    assert_eq!(watchdog(actor.call(Add(4))).await.unwrap(), 7);
    assert_eq!(watchdog(actor.call(Describe)).await.unwrap(), "count=7");
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

struct Events;

impl Message for Events {
    type Reply = mpsc::Receiver<u8>;
}

impl Handler<Events> for Calculator {
    fn handle(
        &mut self,
        _message: Events,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Events> + use<> {
        let (events, receiver) = mpsc::channel(2);
        events.try_send(1).expect("the stream buffer has room");
        events.try_send(2).expect("the stream buffer has room");
        receiver.ready()
    }
}

#[tokio::test]
async fn a_stream_handle_is_an_ordinary_typed_reply() {
    let owner = spawn(Calculator(0));
    let actor = owner.actor_ref();
    let mut events = watchdog(actor.call(Events)).await.unwrap();

    assert_eq!(watchdog(events.recv()).await, Some(1));
    assert_eq!(watchdog(events.recv()).await, Some(2));
    assert_eq!(watchdog(events.recv()).await, None);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

#[derive(Default)]
struct SerialActor {
    committed: Arc<Mutex<Vec<u8>>>,
}

impl Actor for SerialActor {}

struct Block {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for Block {
    type Reply = ();
}

impl Handler<Block> for SerialActor {
    fn handle(
        &mut self,
        message: Block,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Block> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

struct Record(u8);

impl Message for Record {
    type Reply = u8;
}

impl Handler<Record> for SerialActor {
    fn handle(
        &mut self,
        message: Record,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Record> + use<> {
        lock(&self.committed).push(message.0);
        message.0.ready()
    }
}

struct Snapshot;

impl Message for Snapshot {
    type Reply = Vec<u8>;
}

impl Handler<Snapshot> for SerialActor {
    fn handle(
        &mut self,
        _message: Snapshot,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Snapshot> + use<> {
        lock(&self.committed).clone().ready()
    }
}

struct Notify(u8);

impl Message for Notify {
    type Reply = ();
}

impl SyncHandler<Notify> for SerialActor {
    fn handle(&mut self, message: Notify, _scope: &mut ActorScope<Self>) {
        lock(&self.committed).push(message.0);
    }
}

fn single_slot_options() -> SpawnOptions {
    let one = NonZeroUsize::new(1).expect("one is non-zero");
    SpawnOptions::default()
        .with_mailbox_capacity(one)
        .with_max_in_flight(one)
}

#[tokio::test]
async fn try_call_returns_the_original_message_when_mailbox_is_full() {
    let owner = spawn_with(SerialActor::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let queued = actor.try_call(Record(1)).unwrap();
    let full = actor.try_call(Record(2)).unwrap_err();

    assert_eq!(full.kind(), TryCallErrorKind::Full);
    assert_eq!(full.into_message().0, 2);
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(queued).await, Ok(1));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

#[tokio::test]
async fn abandoning_a_queued_response_skips_its_handler() {
    let owner = spawn_with(SerialActor::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let abandoned = actor.try_call(Record(7)).unwrap();
    drop(abandoned);
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));

    assert_eq!(
        watchdog(actor.call(Snapshot)).await.unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

// A one-way envelope must remain dispatchable after admission because there is
// intentionally no Response whose lifetime can keep it alive. Filling the queue
// proves that a waiting send commits when capacity returns; requesting Drain
// immediately afterward proves that commit joined Drain's fixed accepted queue.
#[tokio::test]
async fn admitted_one_way_message_cannot_be_abandoned_by_its_sender() {
    let committed = Arc::new(Mutex::new(Vec::new()));
    let owner = spawn_with(
        SerialActor {
            committed: Arc::clone(&committed),
        },
        single_slot_options(),
    );
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    actor.try_send(Notify(7)).unwrap();
    let mut waiting = Box::pin(actor.send(Notify(8)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(waiting.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());
    assert!(lock(&committed).is_empty());

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    watchdog(waiting).await.unwrap();
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
    assert_eq!(*lock(&committed), vec![7, 8]);
}

// Cancelling a capacity wait cannot commit its message, but unlike a returned
// SendError it also cannot give ownership back. Releasing the actor afterward
// proves that only the already-admitted envelope reaches its handler.
#[tokio::test]
async fn cancelling_a_waiting_send_discards_the_uncommitted_message() {
    let committed = Arc::new(Mutex::new(Vec::new()));
    let owner = spawn_with(
        SerialActor {
            committed: Arc::clone(&committed),
        },
        single_slot_options(),
    );
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    actor.try_send(Notify(1)).unwrap();
    let mut waiting = Box::pin(actor.send(Notify(2)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(waiting.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());
    drop(waiting);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.call(Snapshot)).await.unwrap(), vec![1]);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

// Immediate one-way admission must make retry decisions lossless: both a full
// mailbox and lifecycle cutoff return the exact message that never committed.
#[tokio::test]
async fn try_send_recovers_messages_rejected_as_full_or_closed() {
    let owner = spawn_with(SerialActor::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    actor.try_send(Notify(1)).unwrap();
    let full = actor.try_send(Notify(2)).unwrap_err();
    assert_eq!(full.kind(), TrySendErrorKind::Full);
    assert_eq!(full.into_message().0, 2);

    assert!(matches!(
        owner.request_shutdown(Shutdown::Stop),
        loong_actor::ShutdownStatus::Requested
    ));
    let closed = actor.try_send(Notify(3)).unwrap_err();
    assert_eq!(closed.kind(), TrySendErrorKind::Closed);
    assert_eq!(closed.into_message().0, 3);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.closed()).await, ExitReason::Stopped);
}

// A blocked send owns its message until admission commits, so shutdown must
// wake the waiter and return that message instead of silently discarding it.
#[tokio::test]
async fn send_recovers_a_message_when_shutdown_wins_admission() {
    let owner = spawn_with(SerialActor::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    actor.try_send(Notify(1)).unwrap();
    let mut waiting = Box::pin(actor.send(Notify(2)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(waiting.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());

    assert!(matches!(
        owner.request_shutdown(Shutdown::Stop),
        loong_actor::ShutdownStatus::Requested
    ));
    let rejected = watchdog(waiting).await.unwrap_err();
    assert_eq!(rejected.into_message().0, 2);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.closed()).await, ExitReason::Stopped);
}

struct CommitAfterRelease {
    value: u8,
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for CommitAfterRelease {
    type Reply = ();
}

impl Handler<CommitAfterRelease> for SerialActor {
    fn handle(
        &mut self,
        message: CommitAfterRelease,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, CommitAfterRelease> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            message.value
        }
        .into_actor()
        .map(|value, actor: &mut Self, _scope| {
            lock(&actor.committed).push(value);
        })
        .exclusive()
    }
}

#[tokio::test]
async fn abandoning_an_in_flight_call_does_not_cancel_handler_effects() {
    let owner = spawn(SerialActor::default());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let call = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(CommitAfterRelease {
                    value: 9,
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    call.abort();
    let _ = watchdog(call).await;
    release_tx.send(()).unwrap();

    assert_eq!(watchdog(actor.call(Snapshot)).await.unwrap(), vec![9]);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await,
        ExitReason::Drained
    );
}

#[tokio::test]
async fn a_capacity_waiter_wakes_when_stop_closes_admission() {
    let owner = spawn_with(SerialActor::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let queued = actor.try_call(Record(1)).unwrap();
    let mut waiting = Box::pin(actor.call(Record(2)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(waiting.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());

    assert!(matches!(
        owner.request_shutdown(Shutdown::Stop),
        loong_actor::ShutdownStatus::Requested
    ));
    assert_eq!(watchdog(waiting).await, Err(CallError::Closed));
    assert_eq!(
        actor.try_call(Record(3)).unwrap_err().kind(),
        TryCallErrorKind::Closed
    );

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(
        watchdog(queued).await,
        Err(CallError::BeforeDispatch(ExitReason::Stopped))
    );
    assert_eq!(watchdog(actor.closed()).await, ExitReason::Stopped);
}
