use std::num::NonZeroUsize;

// A serial actor has no interleaved limit to override.
struct Serial;

#[loong_actor::actor(mailbox)]
impl loong_actor::Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Serial>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
