// HasMailbox is derived from transport capabilities.
use loac::{Actor, ActorScope, HasMailbox};

struct Bare;

#[loac::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasMailbox for Bare {}

fn main() {}
