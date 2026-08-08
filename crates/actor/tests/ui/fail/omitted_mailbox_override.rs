use std::num::NonZeroUsize;

struct Bare;

#[loac::actor]
impl loac::Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Bare>::default().with_mailbox_capacity(NonZeroUsize::MIN);
}
