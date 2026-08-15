// Unknown keys must not create an unsupported capability.
struct Unknown;

#[actor_api::actor(queue = unbounded)]
impl actor_api::Actor for Unknown {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
