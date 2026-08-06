use loong_actor::{Actor, ActorScope, HasChildren};

struct Supervisor;

#[loong_actor::actor(children = unbounded)]
impl Actor for Supervisor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasChildren for Supervisor {}

fn main() {}
