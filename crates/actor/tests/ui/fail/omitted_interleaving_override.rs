use std::num::NonZeroUsize;

// A serial actor has no interleaved limit to override.
struct Serial;

#[loac::actor(mailbox)]
impl loac::Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Serial>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
