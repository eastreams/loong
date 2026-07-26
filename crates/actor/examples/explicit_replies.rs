use std::{collections::HashMap, time::Duration};

use loong_actor::{
    Actor, ActorFutureExt, ActorScope, ExitReason, Handler, IntoActorFuture, IntoReply, Message,
    Shutdown, reply, spawn,
};
use tokio::sync::oneshot;

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

struct MultiplyExclusively {
    factor: u64,
    snapshot: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

impl Message for MultiplyExclusively {
    type Reply = u64;
}

impl Handler<MultiplyExclusively> for Store {
    fn handle(
        &mut self,
        message: MultiplyExclusively,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, MultiplyExclusively> + use<> {
        reply::exclusive(
            std::future::ready(message)
                .into_actor()
                .map(|message, actor: &mut Self, _scope| {
                    let _ = message.snapshot.send(());
                    (actor.total, message.factor, message.resume)
                })
                .then(|(total, factor, resume), _actor, _scope| {
                    async move {
                        resume.await.expect("the example retains the resume sender");
                        (total, factor)
                    }
                    .into_actor()
                })
                .map(|(total, factor), actor: &mut Self, _scope| {
                    actor.total = total * factor;
                    actor.total
                }),
        )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Store {
        cache: HashMap::from([("cached".to_owned(), 21)]),
        total: 10,
    });
    let store = owner.actor_ref();

    assert_eq!(store.call(Lookup("cached".to_owned())).await?, 21);
    assert_eq!(store.call(Lookup("uncached".to_owned())).await?, 8);
    assert_eq!(store.call(AddLater(5)).await?, 15);

    let (snapshot_tx, snapshot_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let multiply = store.try_call(MultiplyExclusively {
        factor: 2,
        snapshot: snapshot_tx,
        resume: resume_rx,
    })?;
    snapshot_rx.await?;

    let mut queued_ready_lookup = store.try_call(Lookup("cached".to_owned()))?;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut queued_ready_lookup)
            .await
            .is_err()
    );
    resume_tx
        .send(())
        .expect("the exclusive reply retains the resume receiver");

    assert_eq!(multiply.await?, 30);
    assert_eq!(queued_ready_lookup.await?, 21);
    assert_eq!(owner.shutdown(Shutdown::Drain).await, ExitReason::Drained);
    Ok(())
}
