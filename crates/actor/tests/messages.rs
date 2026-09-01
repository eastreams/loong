#[path = "messages/admission.rs"]
mod admission;
#[path = "messages/capacity_wakers.rs"]
mod capacity_wakers;
#[path = "messages/policies.rs"]
mod policies;
#[path = "messages/stream_reply.rs"]
mod stream_reply;
#[path = "messages/stream_to.rs"]
mod stream_to;
mod support;
#[path = "messages/typed_replies.rs"]
mod typed_replies;

use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use loac::{
    Actor, ActorScope, IntoActorFuture, Message, RawHandler, ReplyExt, SpawnOptions, actor,
};
use tokio::sync::oneshot;

use support::lock;

struct SerialActor {
    committed: Arc<Mutex<Vec<u8>>>,
}

#[actor(mailbox = dynamic)]
impl Actor for SerialActor {
    type SpawnArgs = Arc<Mutex<Vec<u8>>>;

    async fn init(committed: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { committed }
    }
}

#[derive(Message)]
#[message(raw = ())]
struct Block {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl RawHandler<Block> for SerialActor {
    fn handle(
        &mut self,
        message: Block,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Block> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct Record(u8);

impl RawHandler<Record> for SerialActor {
    fn handle(
        &mut self,
        message: Record,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Record> + use<> {
        lock(&self.committed).push(message.0);
        message.0.ready()
    }
}

#[derive(Message)]
#[message(raw = ())]
struct Notify(u8);

impl RawHandler<Notify> for SerialActor {
    fn handle(
        &mut self,
        message: Notify,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Notify> + use<> {
        let __reply = {
            lock(&self.committed).push(message.0);
        };
        __reply.ready()
    }
}

fn single_slot_options() -> SpawnOptions<SerialActor> {
    let one = NonZeroUsize::new(1).expect("one is non-zero");
    SpawnOptions::<SerialActor>::default().with_mailbox_capacity(one)
}
