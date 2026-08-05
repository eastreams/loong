use std::num::NonZeroUsize;

struct Serial;

#[actor_api::actor(mailbox)]
impl actor_api::Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = actor_api::SpawnOptions::<Serial>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
