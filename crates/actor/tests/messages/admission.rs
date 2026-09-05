use std::{
    sync::{Arc, Mutex},
    task::Poll,
};

use loac::{
    ActorFutureExt, ActorScope, CallError, DispatchHandler, ExitReason, IntoActorFuture, Message,
    ReplyExt, Shutdown, TryCallErrorKind, TrySendErrorKind, spawn_with,
};

use super::{
    Block, Notify, Record, SerialActor, single_slot_options,
    support::{lock, watchdog},
};

#[derive(Message)]
#[message(reply = Vec<u8>)]
struct Snapshot;

impl DispatchHandler<Snapshot> for SerialActor {
    fn handle(
        &mut self,
        _message: Snapshot,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Snapshot> + use<> {
        lock(&self.committed).clone().ready()
    }
}

#[derive(Message)]
#[message(reply = ())]
struct CommitAfterRelease {
    value: u8,
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

impl DispatchHandler<CommitAfterRelease> for SerialActor {
    fn handle(
        &mut self,
        message: CommitAfterRelease,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, CommitAfterRelease> + use<> {
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
async fn try_call_returns_the_original_message_when_mailbox_is_full() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

#[tokio::test]
async fn abandoning_a_queued_response_skips_its_handler() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
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
    let owner = spawn_with::<SerialActor>(Arc::clone(&committed), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
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
    let owner = spawn_with::<SerialActor>(Arc::clone(&committed), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

// Cancelling a call's capacity wait cannot commit its message. Releasing the
// actor afterward proves that only the already-admitted envelope reaches its
// handler.
#[tokio::test]
async fn cancelling_a_waiting_call_discards_the_uncommitted_message() {
    let committed = Arc::new(Mutex::new(Vec::new()));
    let owner = spawn_with::<SerialActor>(Arc::clone(&committed), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
    let mut waiting = Box::pin(actor.call(Record(2)));
    let first_poll =
        std::future::poll_fn(|context| Poll::Ready(waiting.as_mut().poll(context))).await;
    assert!(first_poll.is_pending());
    drop(waiting);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.call(Snapshot)).await.unwrap(), vec![1]);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

// Immediate one-way admission must make retry decisions lossless: both a full
// mailbox and lifecycle cutoff return the exact message that never committed.
#[tokio::test]
async fn try_send_recovers_messages_rejected_as_full_or_closed() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        loac::ShutdownStatus::Requested
    ));
    let closed = actor.try_send(Notify(3)).unwrap_err();
    assert_eq!(closed.kind(), TrySendErrorKind::Closed);
    assert_eq!(closed.into_message().0, 3);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Stopped);
}

// Immediate call admission must be lossless too: lifecycle cutoff returns the
// exact message that never committed.
#[tokio::test]
async fn try_call_recovers_the_message_when_shutdown_closes_admission() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
    assert!(matches!(
        owner.request_shutdown(Shutdown::Stop),
        loac::ShutdownStatus::Requested
    ));
    let closed = actor.try_call(Record(2)).unwrap_err();
    assert_eq!(closed.kind(), TryCallErrorKind::Closed);
    assert_eq!(closed.into_message().0, 2);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Stopped);
}

// A blocked send owns its message until admission commits, so shutdown must
// wake the waiter and return that message instead of silently discarding it.
#[tokio::test]
async fn send_recovers_a_message_when_shutdown_wins_admission() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        loac::ShutdownStatus::Requested
    ));
    let rejected = watchdog(waiting).await.unwrap_err();
    assert_eq!(rejected.into_message().0, 2);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Stopped);
}

// A ready capacity permit does not prove lifecycle admission.
// Both async APIs must still pass through the shared gate.
#[tokio::test]
async fn ready_capacity_does_not_bypass_closed_admission() {
    let committed = Arc::default();
    let mut owner = loac::spawn::<SerialActor>(Arc::clone(&committed));
    let actor = owner.actor_ref();

    assert!(matches!(
        owner.request_shutdown(Shutdown::Stop),
        loac::ShutdownStatus::Requested
    ));
    let rejected = actor.send(Notify(1)).await.unwrap_err();
    assert_eq!(rejected.into_message().0, 1);
    assert_eq!(actor.call(Record(2)).await, Err(CallError::Closed));
    assert!(lock(&committed).is_empty());
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Stopped);
}

#[tokio::test]
async fn abandoning_an_in_flight_call_does_not_cancel_handler_effects() {
    let owner = loac::spawn::<SerialActor>(Arc::default());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

#[tokio::test]
async fn a_capacity_waiter_wakes_when_stop_closes_admission() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
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
        loac::ShutdownStatus::Requested
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
    assert_eq!(watchdog(actor.closed()).await.reason(), ExitReason::Stopped);
}
