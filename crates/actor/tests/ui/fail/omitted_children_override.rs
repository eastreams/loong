use std::num::NonZeroUsize;

struct Bare;

#[loong_actor::actor]
impl loong_actor::Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Bare>::default().with_max_children(NonZeroUsize::MIN);
}
