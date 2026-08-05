// Serial actors still support exclusive replies.
use actor_api::prelude::*;

struct Serial;

#[actor(mailbox)]
impl Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
struct Ping;

impl Handler<Ping> for Serial {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, Ping> + use<> {
        async {}.into_actor().exclusive()
    }
}

fn main() {}
