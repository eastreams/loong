use std::sync::{Arc, Mutex};

use loac::{
    Actor, ActorRef, ActorScope, CallError, ExitReason, IntoActorFuture, Message, RawHandler,
    ReplyExt, Shutdown, ShutdownStatus, StopScope, actor,
};
use tokio::sync::oneshot;

use super::support::{lock, watchdog};

struct Worker {
    log: Arc<Mutex<Vec<String>>>,
}

#[actor(mailbox)]
impl Actor for Worker {
    type SpawnArgs = Arc<Mutex<Vec<String>>>;

    async fn init(log: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { log }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("worker-stop".to_owned());
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct Work(u8);

impl RawHandler<Work> for Worker {
    fn handle(
        &mut self,
        message: Work,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Work> + use<> {
        lock(&self.log).push(format!("work-{}", message.0));
        message.0.ready()
    }
}

struct DrainParent {
    log: Arc<Mutex<Vec<String>>>,
    worker: ActorRef<Worker>,
}

struct DrainParentArgs {
    log: Arc<Mutex<Vec<String>>>,
    worker_started: oneshot::Sender<ActorRef<Worker>>,
}

#[actor(mailbox, children = unbounded)]
impl Actor for DrainParent {
    type SpawnArgs = DrainParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(worker) = scope.spawn_child::<Worker>(Arc::clone(&args.log));
        let worker = worker.into_actor_ref();
        let _ = args.worker_started.send(worker.clone());
        Self {
            log: args.log,
            worker,
        }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("parent-stop".to_owned());
    }
}

#[derive(Message)]
#[message(raw = ())]
struct ParentBlock {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl RawHandler<ParentBlock> for DrainParent {
    fn handle(
        &mut self,
        message: ParentBlock,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, ParentBlock> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

#[derive(Message)]
#[message(raw = Result<u8, CallError>)]
struct Forward(u8);

impl RawHandler<Forward> for DrainParent {
    fn handle(
        &mut self,
        message: Forward,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Forward> + use<> {
        let worker = self.worker.clone();
        async move { worker.call(Work(message.0)).await }
    }
}

// Drain finishes the parent queue before draining children.
#[tokio::test]
async fn parent_drain_finishes_parent_queue_before_draining_children() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (worker_tx, worker_rx) = oneshot::channel();
    let mut owner = loac::spawn::<DrainParent>(DrainParentArgs {
        log: log.clone(),
        worker_started: worker_tx,
    });
    let parent = owner.actor_ref();
    let worker = watchdog(worker_rx).await.unwrap();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let parent = parent.clone();
        async move {
            parent
                .call(ParentBlock {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let forwarded = parent.try_call(Forward(7)).unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Drain),
        ShutdownStatus::Requested
    );
    release_tx.send(()).unwrap();

    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(forwarded).await, Ok(Ok(7)));
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Drained);
    assert_eq!(worker.exit_status().unwrap().reason(), ExitReason::Drained);
    assert_eq!(*lock(&log), vec!["work-7", "worker-stop", "parent-stop"]);
}
