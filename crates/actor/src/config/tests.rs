use std::num::NonZeroUsize;

use crate::{Actor, ActorScope, actor};

use super::{
    ActorConfig, DynamicInterleaving, Interleaving, NoInterleaving, UnboundedInterleaving, sealed,
};

struct Bare;

#[actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Fixed;

#[actor(mailbox = 11, interleaved = 7)]
impl Actor for Fixed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Dynamic;

#[actor(mailbox = dynamic, interleaved = dynamic)]
impl Actor for Dynamic {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Unbounded;

#[actor(mailbox = unbounded, interleaved = unbounded)]
impl Actor for Unbounded {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn assert_projection<A, O, I>()
where
    A: ActorConfig,
    A::Messaging: sealed::Messaging<Options = O, Interleaving = I>,
{
}

// Each policy retains only its required spawn state.
// Interleaving remains nested under its messaging policy.
#[test]
fn generated_messaging_policies_project_their_runtime_types() {
    assert_projection::<Bare, (), NoInterleaving>();
    assert_projection::<Fixed, (), Interleaving<7>>();
    assert_projection::<Dynamic, Option<NonZeroUsize>, DynamicInterleaving>();
    assert_projection::<Unbounded, (), UnboundedInterleaving>();
}
