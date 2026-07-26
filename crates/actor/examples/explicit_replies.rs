use std::collections::HashMap;

use loong_actor::{
    Actor, ActorFutureExt, ActorScope, ExitReason, Handler, IntoActorFuture, IntoReply, Message,
    Shutdown, reply, spawn,
};

struct Store {
    cache: HashMap<String, u64>,
    total: u64,
}

impl Actor for Store {}

struct Lookup(String);

impl Message for Lookup {
    type Reply = u64;
}

impl Handler<Lookup> for Store {
    fn handle(
        &mut self,
        message: Lookup,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Lookup> + use<> {
        if let Some(&value) = self.cache.get(&message.0) {
            reply::Either::Left(reply::ready(value))
        } else {
            reply::Either::Right(reply::owned(load_value(message.0)))
        }
    }
}

async fn load_value(key: String) -> u64 {
    tokio::task::yield_now().await;
    key.len() as u64
}

struct AddLater(u64);

impl Message for AddLater {
    type Reply = u64;
}

impl Handler<AddLater> for Store {
    fn handle(
        &mut self,
        message: AddLater,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, AddLater> + use<> {
        reply::interleaved(
            async move {
                tokio::task::yield_now().await;
                message.0
            }
            .into_actor()
            .map(|amount, actor: &mut Self, _scope| {
                actor.total += amount;
                actor.total
            }),
        )
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Store {
        cache: HashMap::from([("cached".to_owned(), 21)]),
        total: 10,
    });
    let store = owner.actor_ref();

    assert_eq!(store.call(Lookup("cached".to_owned())).await?, 21);
    assert_eq!(store.call(Lookup("uncached".to_owned())).await?, 8);
    assert_eq!(store.call(AddLater(5)).await?, 15);
    assert_eq!(owner.shutdown(Shutdown::Drain).await, ExitReason::Drained);
    Ok(())
}
