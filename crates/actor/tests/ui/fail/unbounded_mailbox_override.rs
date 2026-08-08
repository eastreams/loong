use std::num::NonZeroUsize;

struct Unbounded;

#[loac::actor(mailbox = unbounded)]
impl loac::Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Unbounded>::default().with_mailbox_capacity(NonZeroUsize::MIN);
}
