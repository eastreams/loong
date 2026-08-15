// A fixed child set cannot have zero capacity.
struct Zero;

#[actor_api::actor(children = 0)]
impl actor_api::Actor for Zero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
