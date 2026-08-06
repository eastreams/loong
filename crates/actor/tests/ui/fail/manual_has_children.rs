// HasChildren is derived from supervision capabilities.
use loac::{Actor, ActorScope, HasChildren};

struct Bare;

#[loac::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasChildren for Bare {}

fn main() {}
