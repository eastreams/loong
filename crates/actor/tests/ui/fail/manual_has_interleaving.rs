use loac::{Actor, ActorScope, HasInterleaving};

// HasInterleaving is derived from scheduler capability.
struct Serial;

#[loac::actor(mailbox)]
impl Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasInterleaving for Serial {}

fn main() {}
