//! Builds a parent-child address cycle during scope-based initialization.
//! The parent runtime owns the child; both actors keep non-owning addresses.

use loac::{ActorRef, ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};
use tokio::sync::oneshot;

struct Parent {
    child: ActorRef<Child>,
}

#[actor(mailbox, children = unbounded)]
impl Actor for Parent {
    type SpawnArgs = ();

    async fn init(_: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        // The address exists before Parent. The child can retain it immediately.
        let parent = scope.myself().clone();
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
struct Start {
    completed: oneshot::Sender<()>,
}

#[derive(Message)]
struct VisitChild {
    completed: oneshot::Sender<()>,
}

#[derive(Message)]
struct ReturnToParent {
    completed: oneshot::Sender<()>,
}

impl SyncHandler<Start> for Parent {
    fn handle(&mut self, message: Start, _scope: &mut ActorScope<Self>) {
        assert!(
            self.child
                .try_send(VisitChild {
                    completed: message.completed,
                })
                .is_ok()
        );
    }
}

impl SyncHandler<VisitChild> for Child {
    fn handle(&mut self, message: VisitChild, _scope: &mut ActorScope<Self>) {
        assert!(
            self.parent
                .try_send(ReturnToParent {
                    completed: message.completed,
                })
                .is_ok()
        );
    }
}

impl SyncHandler<ReturnToParent> for Parent {
    fn handle(&mut self, message: ReturnToParent, _scope: &mut ActorScope<Self>) {
        let _ = message.completed.send(());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (completed_tx, completed_rx) = oneshot::channel();
    let owner = spawn::<Parent>(());
    let parent = owner.actor_ref();

    parent
        .send(Start {
            completed: completed_tx,
        })
        .await?;
    completed_rx.await?;

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
