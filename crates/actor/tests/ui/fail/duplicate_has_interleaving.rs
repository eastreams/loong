use loong_actor::{Actor, ActorScope, HasInterleaving};

// Scheduler capability supplies this implementation automatically.
struct Interleaved;

#[loong_actor::actor(mailbox, interleaved)]
impl Actor for Interleaved {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasInterleaving for Interleaved {}

fn main() {}
