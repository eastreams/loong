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

impl Actor for LogParent {
    type SpawnArgs = LogParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let child = scope
            .spawn_child::<LogChild>(Arc::clone(&args.log))
            .into_actor_ref();
        let _ = args.child_started.send(child);
        Self { log: args.log }
    }

    async fn on_stop<'a>(&'a mut self, _reason: ExitReason, _scope: &'a mut StopScope<'_, Self>) {
        lock(&self.log).push("parent-stop");
    }
}

#[tokio::test]
async fn parent_stop_cleans_up_children_before_the_parent() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (child_tx, child_rx) = oneshot::channel();
    let owner = spawn::<LogParent>(LogParentArgs {
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
        loong_actor::TryCallErrorKind::Closed
    ));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );

    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Stopped);
    assert_eq!(*lock(&log), vec!["child-stop", "parent-stop"]);
}

#[derive(Message)]
struct ParentPing;

impl Handler<ParentPing> for LogParent {
    fn handle(
        &mut self,
        _message: ParentPing,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, ParentPing> + use<> {
        ().ready()
    }
}

struct Worker {
    log: Arc<Mutex<Vec<String>>>,
}

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
#[message(reply = u8)]
struct Work(u8);

impl Handler<Work> for Worker {
    fn handle(
        &mut self,
        message: Work,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, Work> + use<> {
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

impl Actor for DrainParent {
    type SpawnArgs = DrainParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let worker = scope
            .spawn_child::<Worker>(Arc::clone(&args.log))
            .into_actor_ref();
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
struct ParentBlock {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Handler<ParentBlock> for DrainParent {
    fn handle(
        &mut self,
        message: ParentBlock,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, ParentBlock> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

#[derive(Message)]
#[message(reply = Result<u8, CallError>)]
struct Forward(u8);

impl Handler<Forward> for DrainParent {
    fn handle(
        &mut self,
        message: Forward,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loong_actor::IntoReply<Self, Forward> + use<> {
        let worker = self.worker.clone();
        async move { worker.call(Work(message.0)).await }
    }
}

#[tokio::test]
async fn parent_drain_finishes_parent_queue_before_draining_children() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let (worker_tx, worker_rx) = oneshot::channel();
    let mut owner = spawn::<DrainParent>(DrainParentArgs {
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

struct Leaf {
    drop_entered: Option<oneshot::Sender<()>>,
    drop_release: Option<sync_mpsc::Receiver<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

struct LeafArgs {
    drop_entered: oneshot::Sender<()>,
    drop_release: sync_mpsc::Receiver<()>,
    dropped: oneshot::Sender<()>,
}

impl Actor for Leaf {
    type SpawnArgs = LeafArgs;

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self {
            drop_entered: Some(args.drop_entered),
            drop_release: Some(args.drop_release),
            dropped: Some(args.dropped),
        }
    }
}

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
    dropped: Option<oneshot::Sender<()>>,
}

struct BranchArgs {
    leaf_started: oneshot::Sender<ActorRef<Leaf>>,
    leaf_drop_entered: oneshot::Sender<()>,
    leaf_drop_release: sync_mpsc::Receiver<()>,
    leaf_dropped: oneshot::Sender<()>,
    dropped: oneshot::Sender<()>,
}

impl Actor for Branch {
    type SpawnArgs = BranchArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let leaf = scope
            .spawn_child::<Leaf>(LeafArgs {
                drop_entered: args.leaf_drop_entered,
                drop_release: args.leaf_drop_release,
                dropped: args.leaf_dropped,
            })
            .into_actor_ref();
        let _ = args.leaf_started.send(leaf);
        Self {
            dropped: Some(args.dropped),
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

struct PanicParent;

struct PanicParentArgs {
    branch_started: oneshot::Sender<ActorRef<Branch>>,
    leaf_started: oneshot::Sender<ActorRef<Leaf>>,
    leaf_drop_entered: oneshot::Sender<()>,
    leaf_drop_release: sync_mpsc::Receiver<()>,
    branch_dropped: oneshot::Sender<()>,
    leaf_dropped: oneshot::Sender<()>,
}

impl Actor for PanicParent {
    type SpawnArgs = PanicParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let branch = scope
            .spawn_child::<Branch>(BranchArgs {
                leaf_started: args.leaf_started,
                leaf_drop_entered: args.leaf_drop_entered,
                leaf_drop_release: args.leaf_drop_release,
                leaf_dropped: args.leaf_dropped,
                dropped: args.branch_dropped,
            })
            .into_actor_ref();
        let _ = args.branch_started.send(branch);
        Self
    }
}

#[derive(Message)]
struct PanicTree;

impl Handler<PanicTree> for PanicParent {
    fn handle(
        &mut self,
        _message: PanicTree,
        _scope: &mut ActorScope<'_, Self>,
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
    let mut owner = spawn::<PanicParent>(PanicParentArgs {
        branch_started: branch_tx,
        leaf_started: leaf_tx,
        leaf_drop_entered: leaf_drop_entered_tx,
        leaf_drop_release: leaf_drop_release_rx,
        branch_dropped: branch_dropped_tx,
        leaf_dropped: leaf_dropped_tx,
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

impl Actor for LateChild {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct SpawnDuringInit;

struct SpawnDuringInitArgs {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    spawned: oneshot::Sender<ActorRef<LateChild>>,
}

impl Actor for SpawnDuringInit {
    type SpawnArgs = SpawnDuringInitArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let _ = args.entered.send(());
        let _ = args.release.await;
        let child = scope.spawn_child::<LateChild>(()).into_actor_ref();
        let _ = args.spawned.send(child);
        Self
    }
}

// Shared setup keeps both graceful observations identical.
async fn assert_init_child_joins_graceful_shutdown(shutdown: Shutdown, reason: ExitReason) {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = spawn::<SpawnDuringInit>(SpawnDuringInitArgs {
        entered: entered_tx,
        release: release_rx,
        spawned: spawned_tx,
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

// Drain retains entered init. Its late child joins cleanup.
#[tokio::test]
async fn drain_includes_children_spawned_during_init() {
    assert_init_child_joins_graceful_shutdown(Shutdown::Drain, ExitReason::Drained).await;
}

// Stop retains entered init. Its late child joins cleanup.
#[tokio::test]
async fn stop_includes_children_spawned_during_init() {
    assert_init_child_joins_graceful_shutdown(Shutdown::Stop, ExitReason::Stopped).await;
}

struct SpawnAfterKill;

impl Actor for SpawnAfterKill {
    type SpawnArgs = oneshot::Sender<ActorRef<LateChild>>;

    async fn init(spawned: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        assert_eq!(
            scope.request_shutdown(Shutdown::Kill),
            ShutdownStatus::Requested
        );
        let child = scope.spawn_child::<LateChild>(()).into_actor_ref();
        let _ = spawned.send(child);
        Self
    }
}

// Kill cannot interrupt a poll. Its new child must not escape cleanup.
#[tokio::test]
async fn kill_includes_children_spawned_before_the_current_poll_returns() {
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let mut owner = spawn::<SpawnAfterKill>(spawned_tx);

    let child = watchdog(spawned_rx).await.unwrap();
    let status = watchdog(owner.wait()).await;
    assert_eq!(status.reason(), ExitReason::Killed);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    assert_eq!(child.exit_status().unwrap().reason(), ExitReason::Killed);
}
