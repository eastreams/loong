use std::num::NonZeroUsize;

// An unbounded scheduler has no finite limit to override.
struct Unbounded;

#[loong_actor::actor(mailbox, interleaved = unbounded)]
impl loong_actor::Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Unbounded>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
