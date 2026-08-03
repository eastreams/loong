// MessageActor is derived from mailbox configuration.
use actor_api::{Actor, ActorScope, MessageActor};

struct Bare;

#[actor_api::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl MessageActor for Bare {}

fn main() {}
