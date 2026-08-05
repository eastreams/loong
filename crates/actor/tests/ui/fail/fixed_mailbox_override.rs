use std::num::NonZeroUsize;

// Fixed mailbox capacity cannot change for one spawn.
struct Fixed;

#[loong_actor::actor(mailbox = 8)]
impl loong_actor::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = loong_actor::SpawnOptions::<Fixed>::default()
        .with_mailbox_capacity(NonZeroUsize::MIN);
}
