use std::num::NonZeroUsize;

// A fixed scheduler has no per-spawn limit override.
struct Fixed;

#[loong_actor::actor(mailbox, interleaved = 8)]
impl loong_actor::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Fixed>::default()
        .with_max_in_flight(NonZeroUsize::MIN);
}
