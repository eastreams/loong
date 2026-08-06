use std::num::NonZeroUsize;

struct Fixed;

#[loong_actor::actor(children = 8)]
impl loong_actor::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Fixed>::default().with_max_children(NonZeroUsize::MIN);
}
