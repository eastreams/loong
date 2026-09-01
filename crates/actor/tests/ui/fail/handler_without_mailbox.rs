// A handler requires the actor to expose a public mailbox.
use loac::{Actor, ActorScope, Cx, Handler};

struct Bare;

#[loac::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loac::Message)]
struct Ping;

impl Handler<Ping> for Bare {
    async fn handle(_message: Ping, _cx: Cx<'_, Self>) {}
}

fn main() {}
