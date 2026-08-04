// HasMailbox is derived from transport capabilities.
use actor_api::{Actor, ActorScope, HasMailbox};

struct Bare;

#[actor_api::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasMailbox for Bare {}

fn main() {}
