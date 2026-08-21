use futures_util::StreamExt;
use loac::{
    Actor, ActorScope, ExitReason, InterleavedFutureExt, IntoActorFuture, Message, ReplyExt,
    Shutdown, StreamHandler, actor,
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

impl StreamHandler<StreamNumbers> for StreamActor {
    fn handle<W>(
        &mut self,
        message: StreamNumbers,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, StreamNumbers> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        async move {
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
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

impl StreamHandler<ExclusiveStreamNumbers> for ExclusiveStreamActor {
    fn handle<W>(
        &mut self,
        message: ExclusiveStreamNumbers,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, ExclusiveStreamNumbers> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        async move {
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
        .into_actor()
        .exclusive()
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

impl StreamHandler<InterleavedStreamNumbers> for InterleavedStreamActor {
    fn handle<W>(
        &mut self,
        message: InterleavedStreamNumbers,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, InterleavedStreamNumbers> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        async move {
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
        .into_actor()
        .interleaved()
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

impl StreamHandler<BranchStream> for BranchStreamActor {
    fn handle<W>(
        &mut self,
        message: BranchStream,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, BranchStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        if message.0 == 0 {
            loac::reply::Either::Left(0_u8.ready())
        } else {
            loac::reply::Either::Right(async move {
                for item in 0..message.0 {
                    if out.write(item).await.is_err() {
                        break;
                    }
                }
                message.0
            })
        }
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
