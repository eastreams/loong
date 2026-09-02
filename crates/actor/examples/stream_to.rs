//! Stream messages with a caller-provided writer.
//!
//! The default `call` path creates an item channel and returns `StreamReply`.
//! When the caller already owns a writer, `call_to` and `send_to` pass it
//! straight to the handler: no item channel is created, and `call_to` returns
//! the final value directly.
//!
//! ```console
//! cargo run -p loac --example stream_to
//! ```

use loac::prelude::*;
use tokio::sync::mpsc;

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
        self.items.push(message.0);
        ().ready()
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
        std::mem::take(&mut self.items).ready()
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `call_to` with an mpsc sender: the caller owns the item channel.
    let owner = loac::spawn::<StreamActor>(());

    let (tx, mut rx) = mpsc::channel::<u8>(8);
    let final_value = owner.call_to(StreamNumbers(3), tx).await?;
    assert_eq!(final_value, 3);
    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);
    drain(owner).await;

    // `send_to` is one-way: admission commits, and the final value is dropped.
    let owner = loac::spawn::<StreamActor>(());

    let (tx, mut rx) = mpsc::channel::<u8>(8);
    owner.send_to(StreamNumbers(3), tx).await?;
    assert_eq!(rx.recv().await, Some(0));
    assert_eq!(rx.recv().await, Some(1));
    assert_eq!(rx.recv().await, Some(2));
    assert_eq!(rx.recv().await, None);
    drain(owner).await;

    // `ActorRef` itself is a `Writer`, so items can stream actor-to-actor.
    let stream_owner = loac::spawn::<StreamActor>(());
    let receiver_owner = loac::spawn::<ItemReceiver>(());
    let receiver = receiver_owner.actor_ref();

    let final_value = stream_owner
        .call_to(StreamToActor(3), receiver.clone())
        .await?;
    assert_eq!(final_value, 3);
    let items = receiver_owner.call(Dump).await?;
    assert_eq!(items, vec![0, 1, 2]);

    drain(stream_owner).await;
    drain(receiver_owner).await;

    Ok(())
}

async fn drain<A>(owner: loac::ActorOwner<A>)
where
    A: loac::Actor,
{
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
}
