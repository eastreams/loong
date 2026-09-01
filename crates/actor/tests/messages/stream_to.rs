use loac::{
    Actor, ActorScope, Cx, ExitReason, Message, RawHandler, ReplyExt, Shutdown, StreamHandler,
    actor,
};
use tokio::sync::mpsc;

use super::support::watchdog;

struct StreamActor;

#[actor(mailbox, interleaved = unbounded)]
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
    async fn handle<W>(message: StreamNumbers, mut out: W, _cx: Cx<'_, Self>) -> u8
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        for item in 0..message.0 {
            if out.write(item).await.is_err() {
                break;
            }
        }
        message.0
    }
}

#[derive(Message)]
#[message(raw = ())]
struct Item(u8);

struct ItemReceiver {
    items: Vec<u8>,
}

#[actor(mailbox)]
impl Actor for ItemReceiver {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { items: Vec::new() }
    }
}

impl RawHandler<Item> for ItemReceiver {
    fn handle(
        &mut self,
        message: Item,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Item> + use<> {
        let __reply = {
            self.items.push(message.0);
        };
        __reply.ready()
    }
}

#[derive(Message)]
#[message(raw = Vec<u8>)]
struct Dump;

impl RawHandler<Dump> for ItemReceiver {
    fn handle(
        &mut self,
        _message: Dump,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Dump> + use<> {
        let __reply = { std::mem::take(&mut self.items) };
        __reply.ready()
    }
}

#[derive(Message)]
#[message(stream = Item, reply = u8)]
struct StreamToActor(u8);

impl StreamHandler<StreamToActor> for StreamActor {
    async fn handle<W>(message: StreamToActor, mut out: W, _cx: Cx<'_, Self>) -> u8
    where
        W: loac::Writer<Item> + Send + 'static,
    {
        for value in 0..message.0 {
            if out.write(Item(value)).await.is_err() {
                break;
            }
        }
        message.0
    }
}

async fn drain<A>(owner: loac::ActorOwner<A>)
where
    A: Actor,
{
    let status = watchdog(owner.shutdown(Shutdown::Drain)).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn call_to_writes_to_caller_writer_and_returns_final() {
    let owner = loac::spawn::<StreamActor>(());
    let actor = owner.actor_ref();

    let (tx, mut rx) = mpsc::channel::<u8>(8);
    let final_value = watchdog(actor.call_to(StreamNumbers(3), tx))
        .await
        .expect("the stream call commits");

    assert_eq!(final_value, 3);
    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);

    drain(owner).await;
}

#[tokio::test]
async fn send_to_writes_to_caller_writer_without_waiting_for_final() {
    let owner = loac::spawn::<StreamActor>(());
    let actor = owner.actor_ref();

    let (tx, mut rx) = mpsc::channel::<u8>(8);
    watchdog(actor.send_to(StreamNumbers(3), tx))
        .await
        .expect("the stream send commits");

    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);

    drain(owner).await;
}

#[tokio::test]
async fn call_to_accepts_an_actor_ref_writer() {
    let stream_owner = loac::spawn::<StreamActor>(());
    let stream_actor = stream_owner.actor_ref();
    let receiver_owner = loac::spawn::<ItemReceiver>(());
    let receiver = receiver_owner.actor_ref();

    let final_value = watchdog(stream_actor.call_to(StreamToActor(3), receiver.clone()))
        .await
        .expect("the stream call commits");

    assert_eq!(final_value, 3);
    let items = watchdog(receiver.call(Dump))
        .await
        .expect("the dump call commits");
    assert_eq!(items, vec![0, 1, 2]);

    drain(stream_owner).await;
    drain(receiver_owner).await;
}
