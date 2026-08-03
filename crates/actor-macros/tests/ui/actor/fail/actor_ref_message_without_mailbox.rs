// A bare actor handle exposes lifecycle methods only.
use actor_api::{Actor, ActorRef, ActorScope};

struct Bare;

#[actor_api::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(actor_api::Message)]
struct Ping;

fn lifecycle_only(handle: &ActorRef<Bare>) {
    let _ = handle.exit_status();
    let _ = handle.call(Ping);
}

fn main() {}
