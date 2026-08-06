// HasChildren is derived from supervision capabilities.
use loong_actor::{Actor, ActorScope, HasChildren};

struct Bare;

#[loong_actor::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasChildren for Bare {}

fn main() {}
