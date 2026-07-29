use std::collections::HashMap;

use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};

struct Store {
    cache: HashMap<String, u64>,
}

impl Actor for Store {}

#[derive(Message)]
#[message(reply = u64)]
struct Lookup(String);

impl Handler<Lookup> for Store {
    fn handle(
        &mut self,
        message: Lookup,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Lookup> + use<> {
        if let Some(&value) = self.cache.get(&message.0) {
            reply::Either::Left(value.ready())
        } else {
            reply::Either::Right(load_value(message.0))
        }
    }
}

async fn load_value(key: String) -> u64 {
    tokio::task::yield_now().await;
    key.len() as u64
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Store {
        cache: HashMap::from([("cached".to_owned(), 21)]),
    });
    let store = owner.actor_ref();

    assert_eq!(store.call(Lookup("cached".to_owned())).await?, 21);
    assert_eq!(store.call(Lookup("uncached".to_owned())).await?, 8);
    assert_eq!(owner.shutdown(Shutdown::Drain).await, ExitReason::Drained);
    Ok(())
}
