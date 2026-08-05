use loong_actor::{Actor, ActorScope, HasInterleaving};

// HasInterleaving is derived from scheduler capability.
struct Serial;

#[loong_actor::actor(mailbox)]
impl Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasInterleaving for Serial {}

fn main() {}
