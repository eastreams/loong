use std::num::NonZeroUsize;

struct Fixed;

#[actor_api::actor(mailbox, interleaved = 8)]
impl actor_api::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = actor_api::SpawnOptions::<Fixed>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
