use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use loac::{
    Actor, ActorFutureExt, ActorScope, ExitReason, Handler, IntoActorFuture, Message, ReplyExt,
    SpawnOptions, StopScope, actor, spawn_with,
};
use tokio::sync::oneshot;

use super::support::lock;

pub(super) struct LifecycleActor {
    handled: Arc<Mutex<Vec<u8>>>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

pub(super) struct LifecycleArgs {
    handled: Arc<Mutex<Vec<u8>>>,
    cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

#[actor(mailbox = dynamic)]
impl Actor for LifecycleActor {
    type SpawnArgs = LifecycleArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self {
            handled: args.handled,
            cleanup: args.cleanup,
        }
    }

    async fn on_stop(&mut self, reason: ExitReason, _scope: &mut StopScope<'_, Self>) {
        lock(&self.cleanup).push(reason);
    }
}

#[derive(Message)]
#[message(reply = u8)]
pub(super) struct Step {
    pub(super) id: u8,
    pub(super) entered: Option<oneshot::Sender<()>>,
    pub(super) release: Option<oneshot::Receiver<()>>,
}

impl Step {
    pub(super) fn immediate(id: u8) -> Self {
        Self {
            id,
            entered: None,
            release: None,
        }
    }
}

impl Handler<Step> for LifecycleActor {
    fn handle(
        &mut self,
        mut message: Step,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Step> + use<> {
        async move {
            if let Some(entered) = message.entered.take() {
                let _ = entered.send(());
            }
            if let Some(release) = message.release.take() {
                let _ = release.await;
            }
            message.id
        }
        .into_actor()
        .map(|id, actor: &mut Self, _scope| {
            lock(&actor.handled).push(id);
            id
        })
        .exclusive()
    }
}

pub(super) struct LifecycleHarness {
    pub(super) owner: loac::ActorOwner<LifecycleActor>,
    pub(super) handled: Arc<Mutex<Vec<u8>>>,
    pub(super) cleanup: Arc<Mutex<Vec<ExitReason>>>,
}

pub(super) fn actor_with_capacity(capacity: usize) -> LifecycleHarness {
    let handled = Arc::new(Mutex::new(Vec::new()));
    let cleanup = Arc::new(Mutex::new(Vec::new()));
    let args = LifecycleArgs {
        handled: handled.clone(),
        cleanup: cleanup.clone(),
    };
    let owner = spawn_with::<LifecycleActor>(
        args,
        SpawnOptions::<LifecycleActor>::default()
            .with_mailbox_capacity(NonZeroUsize::new(capacity).expect("test capacity is non-zero")),
    );
    LifecycleHarness {
        owner,
        handled,
        cleanup,
    }
}

pub(super) struct DropSignal(pub(super) Option<oneshot::Sender<()>>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.send(());
        }
    }
}
