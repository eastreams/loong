use std::num::NonZeroUsize;

use loac::{
    Actor, ActorOwner, ActorScope, ExitReason, Message, RawHandler, ReplyExt, Shutdown,
    SpawnOptions, TrySendErrorKind, actor, spawn_with,
};
use tokio::sync::oneshot;

use super::support::watchdog;

#[derive(Message)]
#[message(raw = ())]
struct Ping;

struct DynamicActor;

#[actor(mailbox = dynamic)]
impl Actor for DynamicActor {
    type SpawnArgs = oneshot::Receiver<()>;

    async fn init(release: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = release.await;
        Self
    }
}

impl RawHandler<Ping> for DynamicActor {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        let __reply = {};
        __reply.ready()
    }
}

struct CustomDynamicActor;

#[actor(mailbox = dynamic(3))]
impl Actor for CustomDynamicActor {
    type SpawnArgs = oneshot::Receiver<()>;

    async fn init(release: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = release.await;
        Self
    }
}

impl RawHandler<Ping> for CustomDynamicActor {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        let __reply = {};
        __reply.ready()
    }
}

struct FixedActor;

#[actor(mailbox = 2)]
impl Actor for FixedActor {
    type SpawnArgs = oneshot::Receiver<()>;

    async fn init(release: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = release.await;
        Self
    }
}

impl RawHandler<Ping> for FixedActor {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        let __reply = {};
        __reply.ready()
    }
}

struct UnboundedActor;

#[actor(mailbox = unbounded)]
impl Actor for UnboundedActor {
    type SpawnArgs = oneshot::Receiver<()>;

    async fn init(release: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = release.await;
        Self
    }
}

impl RawHandler<Ping> for UnboundedActor {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        let __reply = {};
        __reply.ready()
    }
}

async fn assert_bounded_capacity<A>(owner: ActorOwner<A>, capacity: usize)
where
    A: RawHandler<Ping>,
{
    let actor = owner.actor_ref();
    for _ in 0..capacity {
        actor.try_send(Ping).expect("the mailbox has room");
    }
    assert_eq!(
        actor.try_send(Ping).unwrap_err().kind(),
        TrySendErrorKind::Full
    );
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Kill)).await.reason(),
        ExitReason::Killed
    );
}

// The key-only dynamic form uses the domain default.
#[tokio::test]
async fn dynamic_mailbox_uses_its_default_capacity() {
    let (_hold_init, init) = oneshot::channel();
    let owner = loac::spawn::<DynamicActor>(init);
    assert_bounded_capacity(owner, 32).await;
}

// `dynamic(N)` changes construction without a spawn override.
#[tokio::test]
async fn dynamic_mailbox_accepts_a_custom_actor_default() {
    let (_hold_init, init) = oneshot::channel();
    let owner = loac::spawn::<CustomDynamicActor>(init);
    assert_bounded_capacity(owner, 3).await;
}

// One spawn can replace its actor-level dynamic default.
#[tokio::test]
async fn dynamic_mailbox_accepts_one_spawn_override() {
    let (_hold_init, init) = oneshot::channel();
    let options = SpawnOptions::<CustomDynamicActor>::default()
        .with_mailbox_capacity(NonZeroUsize::new(2).unwrap());
    let owner = spawn_with::<CustomDynamicActor>(init, options);
    assert_bounded_capacity(owner, 2).await;
}

// Fixed capacity remains entirely in generated actor configuration.
#[tokio::test]
async fn fixed_mailbox_uses_its_actor_capacity() {
    let (_hold_init, init) = oneshot::channel();
    let owner = loac::spawn::<FixedActor>(init);
    assert_bounded_capacity(owner, 2).await;
}

// Unbounded transport must not synthesize saturation.
#[tokio::test]
async fn unbounded_mailbox_never_reports_saturation() {
    let (_hold_init, init) = oneshot::channel();
    let owner = loac::spawn::<UnboundedActor>(init);
    let actor = owner.actor_ref();

    for _ in 0..1_024 {
        actor
            .try_send(Ping)
            .expect("unbounded admission cannot be full");
    }
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Kill)).await.reason(),
        ExitReason::Killed
    );
}
