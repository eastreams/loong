// HasMailbox is derived from transport capabilities.
use loong_actor::{Actor, ActorScope, HasMailbox};

struct Bare;

#[loong_actor::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasMailbox for Bare {}

fn main() {}
