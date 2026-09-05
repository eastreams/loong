use std::sync::{Arc, Mutex};

use loac::{
    Actor, ActorRef, ActorScope, DispatchHandler, ExitReason, Message, ReplyExt, Shutdown,
    ShutdownStatus, StopScope, actor,
};
use tokio::sync::oneshot;

use super::support::{lock, watchdog};

struct LogChild {
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[actor]
impl Actor for LogChild {
    type SpawnArgs = Arc<Mutex<Vec<&'static str>>>;

    async fn init(log: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { log }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("child-stop");
    }
}

struct LogParent {
    log: Arc<Mutex<Vec<&'static str>>>,
}

struct LogParentArgs {
    log: Arc<Mutex<Vec<&'static str>>>,
    child_started: oneshot::Sender<ActorRef<LogChild>>,
}

#[actor(mailbox, children = unbounded)]
impl Actor for LogParent {
    type SpawnArgs = LogParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(child) = scope.spawn_child::<LogChild>(Arc::clone(&args.log));
        let child = child.into_actor_ref();
        let _ = args.child_started.send(child);
        Self { log: args.log }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("parent-stop");
    }
}

#[derive(Message)]
#[message(reply = ())]
struct ParentPing;

impl DispatchHandler<ParentPing> for LogParent {
    fn handle(
        &mut self,
        _message: ParentPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, ParentPing> + use<> {
        ().ready()
    }
}

// Stop cleans children before the parent's terminal event.
#[tokio::test]
async fn parent_stop_cleans_up_children_before_the_parent() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (child_tx, child_rx) = oneshot::channel();
    let owner = loac::spawn::<LogParent>(LogParentArgs {
        log: log.clone(),
        child_started: child_tx,
    });
    let parent = owner.actor_ref();
    let child = watchdog(child_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        ShutdownStatus::Requested
    );
    assert!(matches!(
        parent.try_call(ParentPing).unwrap_err().kind(),
        loac::TryCallErrorKind::Closed
    ));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );

    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Stopped);
    assert_eq!(*lock(&log), vec!["child-stop", "parent-stop"]);
}
