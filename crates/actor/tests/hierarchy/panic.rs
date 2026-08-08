use std::{future::Future, sync::mpsc as sync_mpsc, task::Poll};

use loac::{
    Actor, ActorRef, ActorScope, CallError, ExitReason, Handler, Message, ReplyExt, SubtreeStatus,
    actor,
};
use tokio::sync::oneshot;

use super::support::watchdog;

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

#[actor]
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

#[actor(children = unbounded)]
impl Actor for Branch {
    type SpawnArgs = BranchArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(leaf) = scope.spawn_child::<Leaf>(LeafArgs {
            drop_entered: args.leaf_drop_entered,
            drop_release: args.leaf_drop_release,
            dropped: args.leaf_dropped,
        });
        let leaf = leaf.into_actor_ref();
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

#[actor(mailbox, children = unbounded)]
impl Actor for PanicParent {
    type SpawnArgs = PanicParentArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(branch) = scope.spawn_child::<Branch>(BranchArgs {
            leaf_started: args.leaf_started,
            leaf_drop_entered: args.leaf_drop_entered,
            leaf_drop_release: args.leaf_drop_release,
            leaf_dropped: args.leaf_dropped,
            dropped: args.branch_dropped,
        });
        let branch = branch.into_actor_ref();
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
    ) -> impl loac::IntoReply<Self, PanicTree> + use<> {
        panic!("intentional parent panic");
        #[allow(unreachable_code)]
        ().ready()
    }
}

// Parent panic kills descendants before publishing parent exit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_panic_kills_descendants_before_parent_exit() {
    let (branch_tx, branch_rx) = oneshot::channel();
    let (leaf_tx, leaf_rx) = oneshot::channel();
    let (leaf_drop_entered_tx, leaf_drop_entered_rx) = oneshot::channel();
    let (leaf_drop_release_tx, leaf_drop_release_rx) = sync_mpsc::channel();
    let (branch_dropped_tx, branch_dropped_rx) = oneshot::channel();
    let (leaf_dropped_tx, leaf_dropped_rx) = oneshot::channel();
    let mut owner = loac::spawn::<PanicParent>(PanicParentArgs {
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
