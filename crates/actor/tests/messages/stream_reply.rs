use futures_util::StreamExt;
use loac::{
    Actor, ActorScope, DispatchHandler, ExitReason, InterleavedFutureExt, IntoActorFuture, Message,
    ReplyExt, Shutdown, StreamKind, Writer, actor,
};

use super::support::watchdog;

struct StreamActor;

#[actor(mailbox)]
impl Actor for StreamActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct StreamNumbers(u8);

impl DispatchHandler<StreamNumbers, StreamKind> for StreamActor {
    fn handle(
        &mut self,
        message: StreamNumbers,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, StreamNumbers> + use<> {
        let (item_tx, item_rx) = tokio::sync::mpsc::channel::<u8>(8);
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<u8>();
        let strategy = async move {
            let mut out = item_tx;
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        };
        loac::StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

struct ExclusiveStreamActor;

#[actor(mailbox)]
impl Actor for ExclusiveStreamActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct ExclusiveStreamNumbers(u8);

impl DispatchHandler<ExclusiveStreamNumbers, StreamKind> for ExclusiveStreamActor {
    fn handle(
        &mut self,
        message: ExclusiveStreamNumbers,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, ExclusiveStreamNumbers> + use<> {
        let (item_tx, item_rx) = tokio::sync::mpsc::channel::<u8>(8);
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<u8>();
        let strategy = async move {
            let mut out = item_tx;
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
        .into_actor()
        .exclusive();
        loac::StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

struct InterleavedStreamActor;

#[actor(mailbox, interleaved)]
impl Actor for InterleavedStreamActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct InterleavedStreamNumbers(u8);

impl DispatchHandler<InterleavedStreamNumbers, StreamKind> for InterleavedStreamActor {
    fn handle(
        &mut self,
        message: InterleavedStreamNumbers,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, InterleavedStreamNumbers> + use<> {
        let (item_tx, item_rx) = tokio::sync::mpsc::channel::<u8>(8);
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<u8>();
        let strategy = async move {
            let mut out = item_tx;
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
        .into_actor()
        .interleaved();
        loac::StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

struct BranchStreamActor;

#[actor(mailbox)]
impl Actor for BranchStreamActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct BranchStream(u8);

impl DispatchHandler<BranchStream, StreamKind> for BranchStreamActor {
    fn handle(
        &mut self,
        message: BranchStream,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, BranchStream> + use<> {
        let (item_tx, item_rx) = tokio::sync::mpsc::channel::<u8>(8);
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<u8>();
        let strategy = if message.0 == 0 {
            loac::reply::Either::Left(0_u8.ready())
        } else {
            loac::reply::Either::Right(async move {
                let mut out = item_tx;
                for item in 0..message.0 {
                    if out.write(item).await.is_err() {
                        break;
                    }
                }
                message.0
            })
        };
        loac::StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

#[tokio::test]
async fn stream_handler_streams_items_and_finishes() {
    let owner = loac::spawn::<StreamActor>(());
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(StreamNumbers(3)))
        .await
        .expect("the stream call commits");

    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, Some(2));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await, Ok(3));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn finish_discards_buffered_items_and_returns_final() {
    let owner = loac::spawn::<StreamActor>(());
    let actor = owner.actor_ref();

    let reply = watchdog(actor.call(StreamNumbers(3)))
        .await
        .expect("the stream call commits");

    assert_eq!(reply.finish().await, Ok(3));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn items_view_borrows_without_losing_final() {
    let owner = loac::spawn::<StreamActor>(());
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(StreamNumbers(3)))
        .await
        .expect("the stream call commits");

    {
        let mut items = reply.items();
        assert_eq!(items.next().await, Some(0));
        assert_eq!(items.next().await, Some(1));
        assert_eq!(items.next().await, Some(2));
        assert_eq!(items.next().await, None);
    }

    assert_eq!(reply.finish().await, Ok(3));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn stream_handler_exclusive_strategy_streams_and_finishes() {
    let owner = loac::spawn::<ExclusiveStreamActor>(());
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(ExclusiveStreamNumbers(3)))
        .await
        .expect("the stream call commits");

    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, Some(2));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await, Ok(3));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn stream_handler_interleaved_strategy_streams_and_finishes() {
    let owner = loac::spawn::<InterleavedStreamActor>(());
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(InterleavedStreamNumbers(3)))
        .await
        .expect("the stream call commits");

    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, Some(2));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await, Ok(3));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn stream_handler_either_strategy_chooses_branch() {
    let owner = loac::spawn::<BranchStreamActor>(());
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(BranchStream(0)))
        .await
        .expect("the stream call commits");
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await, Ok(0));

    let mut reply = watchdog(actor.call(BranchStream(2)))
        .await
        .expect("the stream call commits");
    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await, Ok(2));

    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}
