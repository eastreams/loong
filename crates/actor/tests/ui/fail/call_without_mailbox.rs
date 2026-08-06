// A no-mailbox actor cannot call or send through its ref.
use loac::{Actor, ActorScope, spawn};

struct Bare;

#[loac::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loac::Message)]
struct Ping;

fn main() {
    let owner = spawn::<Bare>(());
    let actor_ref = owner.actor_ref();
    let _ = actor_ref.call(Ping);
    let _ = actor_ref.send(Ping);
}
