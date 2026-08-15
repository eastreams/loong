// Repeated options must not silently select one interleaved limit.
struct Duplicate;

#[actor_api::actor(interleaved = 8, interleaved = unbounded)]
impl actor_api::Actor for Duplicate {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
