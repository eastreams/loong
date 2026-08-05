use std::num::NonZeroUsize;

struct Unbounded;

#[actor_api::actor(mailbox, interleaved = unbounded)]
impl actor_api::Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = actor_api::SpawnOptions::<Unbounded>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
