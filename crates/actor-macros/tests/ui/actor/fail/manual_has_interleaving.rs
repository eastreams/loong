use actor_api::{Actor, ActorScope, HasInterleaving};

struct Serial;

#[actor_api::actor(mailbox)]
impl Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasInterleaving for Serial {}

fn main() {}
