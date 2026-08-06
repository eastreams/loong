use std::num::NonZeroUsize;

// A fixed scheduler has no per-spawn limit override.
struct Fixed;

#[loac::actor(mailbox, interleaved = 8)]
impl loac::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Fixed>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
