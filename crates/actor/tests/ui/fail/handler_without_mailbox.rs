// A handler requires the actor to expose a public mailbox.
use loac::{Actor, ActorScope, Handler, ReplyExt};

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
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        ().ready()
    }
}

fn main() {}
