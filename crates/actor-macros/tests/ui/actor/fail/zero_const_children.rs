// Named constants must fail before actor spawning can panic.
const ZERO: usize = 0;

struct Zero;

#[actor_api::actor(children = ZERO)]
impl actor_api::Actor for Zero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
