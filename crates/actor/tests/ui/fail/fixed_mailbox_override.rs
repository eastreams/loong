use std::num::NonZeroUsize;

// Fixed mailbox capacity cannot change for one spawn.
struct Fixed;

#[loac::actor(mailbox = 8)]
impl loac::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loac::SpawnOptions::<Fixed>::default()
        .with_mailbox_capacity(NonZeroUsize::MIN);
}
