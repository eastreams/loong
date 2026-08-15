// Interleaved replies require a mailbox for dispatch.
struct Interleaved;

#[actor_api::actor(interleaved = 4)]
impl actor_api::Actor for Interleaved {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
