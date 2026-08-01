mod support;

use std::{
    future::Future,
    sync::{Arc, Mutex, mpsc as sync_mpsc},
    task::Poll,
};

use loong_actor::{
    Actor, ActorRef, ActorScope, CallError, ExitReason, Handler, IntoActorFuture, Message,
    ReplyExt, Shutdown, ShutdownStatus, StopScope, SubtreeStatus, spawn,
};
use tokio::sync::oneshot;

use support::{lock, watchdog};

struct LogChild {
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Actor for LogChild {
    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("child-stop");
    }
}

struct LogParent {
    log: Arc<Mutex<Vec<&'static str>>>,
    child_started: Option<oneshot::Sender<ActorRef<LogChild>>>,
}

impl Actor for LogParent {
    async fn on_start<'a>(&'a mut self, scope: &'a mut ActorScope<'_, Self>) {
        let child = scope.spawn_child(LogChild {
            log: self.log.clone(),
        });
        if let Some(started) = self.child_started.take() {
            let _ = started.send(child.into_actor_ref());
        }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("parent-stop");
    }
}

#[tokio::test]
async fn parent_stop_cleans_up_children_before_the_parent() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (child_tx, child_rx) = oneshot::channel();
    let owner = spawn(LogParent {
        log: log.clone(),
        child_started: Some(child_tx),
    });
    let parent = owner.actor_ref();
    let child = watchdog(child_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        ShutdownStatus::Requested
    );
    assert!(matches!(
        parent.try_call(ParentPing).unwrap_err().kind(),
        loong_actor::TryCallErrorKind::Closed
    ));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );

    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Stopped);
    assert_eq!(*lock(&log), vec!["child-stop", "parent-stop"]);
}

struct ParentPing;

impl Message for ParentPing {
    type Reply = ();
}

impl Handler<ParentPing> for LogParent {
    fn handle(
        &mut self,
        _message: ParentPing,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ParentPing> + use<> {
        ().ready()
    }
}

struct Worker {
    log: Arc<Mutex<Vec<String>>>,
}

impl Actor for Worker {
    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("worker-stop".to_owned());
    }
}

struct Work(u8);

impl Message for Work {
    type Reply = u8;
}

impl Handler<Work> for Worker {
    fn handle(
        &mut self,
        message: Work,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Work> + use<> {
        lock(&self.log).push(format!("work-{}", message.0));
        message.0.ready()
    }
}

struct DrainParent {
    log: Arc<Mutex<Vec<String>>>,
    worker: Option<ActorRef<Worker>>,
    worker_started: Option<oneshot::Sender<ActorRef<Worker>>>,
}

impl Actor for DrainParent {
    async fn on_start<'a>(&'a mut self, scope: &'a mut ActorScope<'_, Self>) {
        let worker = scope
            .spawn_child(Worker {
                log: self.log.clone(),
            })
            .into_actor_ref();
        self.worker = Some(worker.clone());
        if let Some(started) = self.worker_started.take() {
            let _ = started.send(worker);
        }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("parent-stop".to_owned());
    }
}

struct ParentBlock {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for ParentBlock {
    type Reply = ();
}

impl Handler<ParentBlock> for DrainParent {
    fn handle(
        &mut self,
        message: ParentBlock,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ParentBlock> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

struct Forward(u8);

impl Message for Forward {
    type Reply = Result<u8, CallError>;
}

impl Handler<Forward> for DrainParent {
    fn handle(
        &mut self,
        message: Forward,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Forward> + use<> {
        let worker = self
            .worker
            .as_ref()
            .expect("on_start installs the worker")
            .clone();
        async move { worker.call(Work(message.0)).await }
    }
}

#[tokio::test]
async fn parent_drain_finishes_parent_queue_before_draining_children() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (worker_tx, worker_rx) = oneshot::channel();
    let mut owner = spawn(DrainParent {
        log: log.clone(),
        worker: None,
        worker_started: Some(worker_tx),
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

struct Leaf {
    drop_entered: Option<oneshot::Sender<()>>,
    drop_release: Option<sync_mpsc::Receiver<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Actor for Leaf {}

impl Drop for Leaf {
    fn drop(&mut self) {
        if let Some(entered) = self.drop_entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.drop_release.take() {
            let _ = release.recv();
        }
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

struct Branch {
    leaf_started: Option<oneshot::Sender<ActorRef<Leaf>>>,
    leaf_drop_entered: Option<oneshot::Sender<()>>,
    leaf_drop_release: Option<sync_mpsc::Receiver<()>>,
    leaf_dropped: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Actor for Branch {
    async fn on_start<'a>(&'a mut self, scope: &'a mut ActorScope<'_, Self>) {
        let leaf = scope
            .spawn_child(Leaf {
                drop_entered: self.leaf_drop_entered.take(),
                drop_release: self.leaf_drop_release.take(),
                dropped: self.leaf_dropped.take(),
            })
            .into_actor_ref();
        if let Some(started) = self.leaf_started.take() {
            let _ = started.send(leaf);
        }
    }
}

impl Drop for Branch {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

struct PanicParent {
    branch_started: Option<oneshot::Sender<ActorRef<Branch>>>,
    leaf_started: Option<oneshot::Sender<ActorRef<Leaf>>>,
    leaf_drop_entered: Option<oneshot::Sender<()>>,
    leaf_drop_release: Option<sync_mpsc::Receiver<()>>,
    branch_dropped: Option<oneshot::Sender<()>>,
    leaf_dropped: Option<oneshot::Sender<()>>,
}

impl Actor for PanicParent {
    async fn on_start<'a>(&'a mut self, scope: &'a mut ActorScope<'_, Self>) {
        let branch = scope
            .spawn_child(Branch {
                leaf_started: self.leaf_started.take(),
                leaf_drop_entered: self.leaf_drop_entered.take(),
                leaf_drop_release: self.leaf_drop_release.take(),
                leaf_dropped: self.leaf_dropped.take(),
                dropped: self.branch_dropped.take(),
            })
            .into_actor_ref();
        if let Some(started) = self.branch_started.take() {
            let _ = started.send(branch);
        }
    }
}

struct PanicTree;

impl Message for PanicTree {
    type Reply = ();
}

impl Handler<PanicTree> for PanicParent {
    fn handle(
        &mut self,
        _message: PanicTree,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PanicTree> + use<> {
        panic!("intentional parent panic");
        #[allow(unreachable_code)]
        ().ready()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_panic_kills_descendants_before_parent_exit() {
    let (branch_tx, branch_rx) = oneshot::channel();
    let (leaf_tx, leaf_rx) = oneshot::channel();
    let (leaf_drop_entered_tx, leaf_drop_entered_rx) = oneshot::channel();
    let (leaf_drop_release_tx, leaf_drop_release_rx) = sync_mpsc::channel();
    let (branch_dropped_tx, branch_dropped_rx) = oneshot::channel();
    let (leaf_dropped_tx, leaf_dropped_rx) = oneshot::channel();
    let mut owner = spawn(PanicParent {
        branch_started: Some(branch_tx),
        leaf_started: Some(leaf_tx),
        leaf_drop_entered: Some(leaf_drop_entered_tx),
        leaf_drop_release: Some(leaf_drop_release_rx),
        branch_dropped: Some(branch_dropped_tx),
        leaf_dropped: Some(leaf_dropped_tx),
    });
    let parent = owner.actor_ref();
    let branch = watchdog(branch_rx).await.unwrap();
    let leaf = watchdog(leaf_rx).await.unwrap();

    assert_eq!(
        watchdog(parent.call(PanicTree)).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    watchdog(leaf_drop_entered_rx).await.unwrap();

    let mut parent_exit = Box::pin(owner.wait());
    let first_poll =
        std::future::poll_fn(|task| Poll::Ready(parent_exit.as_mut().poll(task))).await;
    assert!(first_poll.is_pending());

    leaf_drop_release_tx.send(()).unwrap();
    let parent_status = watchdog(parent_exit).await;
    assert_eq!(parent_status.reason(), ExitReason::Panicked);
    assert_eq!(parent_status.subtree(), SubtreeStatus::Terminated);

    let branch_status = branch.exit_status().unwrap();
    assert_eq!(branch_status.reason(), ExitReason::Killed);
    assert_eq!(branch_status.subtree(), SubtreeStatus::Terminated);
    let leaf_status = leaf.exit_status().unwrap();
    assert_eq!(leaf_status.reason(), ExitReason::Killed);
    assert_eq!(leaf_status.subtree(), SubtreeStatus::Terminated);
    watchdog(branch_dropped_rx).await.unwrap();
    watchdog(leaf_dropped_rx).await.unwrap();
}

struct LateChild;

impl Actor for LateChild {}

struct SpawnDuringStartup {
    entered: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    spawned: Option<oneshot::Sender<ActorRef<LateChild>>>,
}

impl Actor for SpawnDuringStartup {
    async fn on_start(&mut self, scope: &mut ActorScope<'_, Self>) {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = self.release.take() {
            let _ = release.await;
        }
        let child = scope.spawn_child(LateChild).into_actor_ref();
        if let Some(spawned) = self.spawned.take() {
            let _ = spawned.send(child);
        }
    }
}

async fn assert_startup_child_joins_graceful_shutdown(shutdown: Shutdown, reason: ExitReason) {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = spawn(SpawnDuringStartup {
        entered: Some(entered_tx),
        release: Some(release_rx),
        spawned: Some(spawned_tx),
    });

    watchdog(entered_rx).await.unwrap();
    assert_eq!(owner.request_shutdown(shutdown), ShutdownStatus::Requested);
    release_tx.send(()).unwrap();
    let child = watchdog(spawned_rx).await.unwrap();

    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), reason);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), reason);
}

// Drain retains entered startup. Its late child must join Drain cleanup.
#[tokio::test]
async fn drain_includes_children_spawned_by_retained_work() {
    assert_startup_child_joins_graceful_shutdown(Shutdown::Drain, ExitReason::Drained).await;
}

// Stop also retains entered startup. Its late child must join Stop cleanup.
#[tokio::test]
async fn stop_includes_children_spawned_by_retained_work() {
    assert_startup_child_joins_graceful_shutdown(Shutdown::Stop, ExitReason::Stopped).await;
}

struct SpawnAfterKill {
    spawned: Option<oneshot::Sender<ActorRef<LateChild>>>,
}

impl Actor for SpawnAfterKill {
    async fn on_start(&mut self, scope: &mut ActorScope<'_, Self>) {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
        let child = scope.spawn_child(LateChild).into_actor_ref();
        if let Some(spawned) = self.spawned.take() {
            let _ = spawned.send(child);
        }
    }
}

// Kill cannot interrupt a poll. Its new child must not escape cleanup.
#[tokio::test]
async fn kill_includes_children_spawned_before_the_current_poll_returns() {
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = spawn(SpawnAfterKill {
        spawned: Some(spawned_tx),
    });

    let child = watchdog(spawned_rx).await.unwrap();
    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), ExitReason::Killed);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Killed);
}
