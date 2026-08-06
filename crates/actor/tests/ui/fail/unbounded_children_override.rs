use std::num::NonZeroUsize;

struct Unbounded;

#[loong_actor::actor(children = unbounded)]
impl loong_actor::Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Unbounded>::default().with_max_children(NonZeroUsize::MIN);
}
