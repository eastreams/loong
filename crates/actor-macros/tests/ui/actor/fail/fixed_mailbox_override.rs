use std::num::NonZeroUsize;

// Fixed mailbox capacity cannot change for one spawn.
struct Fixed;

#[actor_api::actor(mailbox = 8)]
impl actor_api::Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let _ = actor_api::SpawnOptions::<Fixed>::default()
        .with_mailbox_capacity(NonZeroUsize::MIN);
}
