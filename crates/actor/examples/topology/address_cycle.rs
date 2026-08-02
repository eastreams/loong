//! Builds a parent-child address cycle during context-first initialization.
//! The parent runtime owns the child; both actors keep non-owning addresses.

use loong_actor::{ActorRef, ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};
use tokio::sync::oneshot;

struct Parent {
    child: ActorRef<Child>,
    completed: Option<oneshot::Sender<()>>,
}

impl Actor for Parent {
    type SpawnArgs = oneshot::Sender<()>;

    async fn init(completed: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        // The address exists before Parent. The child can retain it immediately.
        let parent = scope.myself().clone();
        let child = scope.spawn_child::<Child>(parent).into_actor_ref();

        Self {
            child,
            completed: Some(completed),
        }
    }
}

struct Child {
    parent: ActorRef<Parent>,
}

impl Actor for Child {
    type SpawnArgs = ActorRef<Parent>;

    async fn init(parent: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { parent }
    }
}

#[derive(Message)]
struct Start;

#[derive(Message)]
struct VisitChild;

#[derive(Message)]
struct ReturnToParent;

impl SyncHandler<Start> for Parent {
    fn handle(&mut self, _message: Start, _scope: &mut ActorScope<Self>) {
        assert!(self.child.try_send(VisitChild).is_ok());
    }
}

impl SyncHandler<VisitChild> for Child {
    fn handle(&mut self, _message: VisitChild, _scope: &mut ActorScope<Self>) {
        assert!(self.parent.try_send(ReturnToParent).is_ok());
    }
}

impl SyncHandler<ReturnToParent> for Parent {
    fn handle(&mut self, _message: ReturnToParent, _scope: &mut ActorScope<Self>) {
        let completed = self.completed.take().expect("the cycle completes once");
        let _ = completed.send(());
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (completed_tx, completed_rx) = oneshot::channel();
    let owner = spawn::<Parent>(completed_tx);
    let parent = owner.actor_ref();

    parent.send(Start).await?;
    completed_rx.await?;

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
