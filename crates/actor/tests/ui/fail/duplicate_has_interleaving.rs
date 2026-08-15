use loac::{Actor, ActorScope, HasInterleaving};

// Scheduler capability supplies this implementation automatically.
struct Interleaved;

#[loac::actor(mailbox, interleaved)]
impl Actor for Interleaved {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasInterleaving for Interleaved {}

fn main() {}
