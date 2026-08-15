use std::num::NonZeroUsize;

// An unbounded scheduler has no finite limit to override.
struct Unbounded;

#[loac::actor(mailbox, interleaved = unbounded)]
impl loac::Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Unbounded>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
