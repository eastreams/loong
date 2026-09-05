//! Builds a parent-child address cycle during scope-based initialization.
//! The parent runtime owns the child; both actors keep non-owning addresses.

use loac::{ActorRef, DispatchHandler, ReplyExt, prelude::*};
use tokio::sync::oneshot;

struct Parent {
    child: ActorRef<Child>,
}

#[actor(mailbox, children = unbounded)]
impl Actor for Parent {
    type SpawnArgs = ();

    async fn init(_: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        // The address exists before Parent. The child can retain it immediately.
        let parent = scope.clone();
        let Ok(child) = scope.spawn_child::<Child>(parent);
        let child = child.into_actor_ref();

        Self { child }
    }
}

struct Child {
    parent: ActorRef<Parent>,
}

#[actor(mailbox)]
impl Actor for Child {
    type SpawnArgs = ActorRef<Parent>;

    async fn init(parent: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { parent }
    }
}

#[derive(Message)]
#[message(reply = ())]
struct Start {
    completed: oneshot::Sender<()>,
}

#[derive(Message)]
#[message(reply = ())]
struct VisitChild {
    completed: oneshot::Sender<()>,
}

#[derive(Message)]
#[message(reply = ())]
struct ReturnToParent {
    completed: oneshot::Sender<()>,
}

impl DispatchHandler<Start> for Parent {
    fn handle(
        &mut self,
        message: Start,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Start> + use<> {
        assert!(
            self.child
                .try_send(VisitChild {
                    completed: message.completed,
                })
                .is_ok()
        );
        ().ready()
    }
}

impl DispatchHandler<VisitChild> for Child {
    fn handle(
        &mut self,
        message: VisitChild,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, VisitChild> + use<> {
        assert!(
            self.parent
                .try_send(ReturnToParent {
                    completed: message.completed,
                })
                .is_ok()
        );
        ().ready()
    }
}

impl DispatchHandler<ReturnToParent> for Parent {
    fn handle(
        &mut self,
        message: ReturnToParent,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ReturnToParent> + use<> {
        let _ = message.completed.send(());
        ().ready()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (completed_tx, completed_rx) = oneshot::channel();
    let owner = loac::spawn::<Parent>(());

    owner
        .send(Start {
            completed: completed_tx,
        })
        .await?;
    completed_rx.await?;

    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    assert_eq!(status.subtree(), loac::SubtreeStatus::Terminated);
    Ok(())
}
