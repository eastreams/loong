// A handler requires the actor to expose a public mailbox.
use loong_actor::{Actor, ActorScope, Handler, ReplyExt};

struct Bare;

#[loong_actor::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loong_actor::Message)]
struct Ping;

impl Handler<Ping> for Bare {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, Ping> + use<> {
        ().ready()
    }
}

fn main() {}
