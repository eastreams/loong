// A handler requires the actor to expose a public mailbox.
use actor_api::{Actor, ActorScope, Handler, ReplyExt};

struct Bare;

#[actor_api::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(actor_api::Message)]
struct Ping;

impl Handler<Ping> for Bare {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl actor_api::IntoReply<Self, Ping> + use<> {
        ().ready()
    }
}

fn main() {}
