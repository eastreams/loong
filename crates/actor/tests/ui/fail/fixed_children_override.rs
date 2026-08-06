use std::num::NonZeroUsize;

struct Fixed;

#[loac::actor(children = 8)]
impl loac::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Fixed>::default().with_max_children(NonZeroUsize::MIN);
}
