use loac::{Actor, ActorScope};

struct Child;

#[loac::actor]
impl Actor for Child {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Parent;

#[loac::actor]
impl Actor for Parent {
    type SpawnArgs = ();

    async fn init(_: (), scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = scope.spawn_child::<Child>(());
        Self
    }
}

fn main() {}
