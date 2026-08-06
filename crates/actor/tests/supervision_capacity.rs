mod support;

use std::{
    convert::Infallible,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use loong_actor::{
    Actor, ActorConfig, ActorScope, MessageConfig, Shutdown, SpawnOptions, SupervisionConfig,
    actor, scheduling, spawn, spawn_with, supervision,
    transport::{NoInbox, NoSender},
};
use tokio::sync::oneshot;

use support::watchdog;

struct IdleChild;

#[actor]
impl Actor for IdleChild {
    type SpawnArgs = usize;

    async fn init(_: usize, _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct FixedParent;

#[actor(children = 1)]
impl Actor for FixedParent {
    type SpawnArgs = oneshot::Sender<usize>;

    async fn init(result: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(_first) = scope.spawn_child::<IdleChild>(1) else {
            panic!("the first child must fit");
        };
        let Err(full) = scope.spawn_child::<IdleChild>(2) else {
            panic!("fixed supervision exceeded its capacity");
        };
        let _ = result.send(full.into_inner());
        Self
    }
}

// Finite rejection returns the untouched child arguments.
#[tokio::test(flavor = "current_thread")]
async fn fixed_capacity_returns_rejected_spawn_args() {
    let (result_tx, result_rx) = oneshot::channel();
    let owner = spawn::<FixedParent>(result_tx);

    assert_eq!(watchdog(result_rx).await.unwrap(), 2);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        loong_actor::ExitReason::Stopped,
    );
}

struct DynamicParent;

#[actor(children = dynamic(2))]
impl Actor for DynamicParent {
    type SpawnArgs = oneshot::Sender<(usize, Vec<usize>)>;

    async fn init(result: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let mut accepted = 0;
        let mut rejected = Vec::new();
        for id in 0..4 {
            match scope.spawn_child::<IdleChild>(id) {
                Ok(_) => accepted += 1,
                Err(full) => rejected.push(full.into_inner()),
            }
        }
        let _ = result.send((accepted, rejected));
        Self
    }
}

// Dynamic capacity uses its declared default and per-spawn override.
#[tokio::test(flavor = "current_thread")]
async fn dynamic_capacity_is_enforced_at_runtime() {
    let (default_tx, default_rx) = oneshot::channel();
    let default_owner = spawn::<DynamicParent>(default_tx);
    assert_eq!(watchdog(default_rx).await.unwrap(), (2, vec![2, 3]));
    let _ = watchdog(default_owner.shutdown(Shutdown::Stop)).await;

    let (override_tx, override_rx) = oneshot::channel();
    let options = SpawnOptions::<DynamicParent>::default().with_max_children(NonZeroUsize::MIN);
    let override_owner = spawn_with::<DynamicParent>(override_tx, options);
    assert_eq!(watchdog(override_rx).await.unwrap(), (1, vec![1, 2, 3]));
    let _ = watchdog(override_owner.shutdown(Shutdown::Stop)).await;
}

struct UnboundedParent;

#[actor(children = unbounded)]
impl Actor for UnboundedParent {
    type SpawnArgs = oneshot::Sender<usize>;

    async fn init(result: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        const CHILDREN: usize = 64;
        for id in 0..CHILDREN {
            let _child = unwrap_infallible(scope.spawn_child::<IdleChild>(id));
        }
        let _ = result.send(CHILDREN);
        Self
    }
}

// This signature proves the public error is exactly Infallible.
fn unwrap_infallible<T>(result: Result<T, Infallible>) -> T {
    match result {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unbounded_supervision_accepts_beyond_the_default_limit() {
    let (result_tx, result_rx) = oneshot::channel();
    let owner = spawn::<UnboundedParent>(result_tx);

    assert_eq!(watchdog(result_rx).await.unwrap(), 64);
    let _ = watchdog(owner.shutdown(Shutdown::Stop)).await;
}

#[derive(Clone)]
struct ProbeOptions {
    opened: Arc<AtomicUsize>,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            opened: Arc::new(AtomicUsize::new(0)),
        }
    }
}

struct PreparationProbe;

impl ActorConfig for PreparationProbe {
    type Options = ProbeOptions;
}

impl MessageConfig for PreparationProbe {
    type Sender = NoSender;
    type Inbox = NoInbox;
    type Scheduler = scheduling::Disabled;

    fn open(options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        options.opened.fetch_add(1, Ordering::SeqCst);
        let (sender, inbox) = NoSender::open();
        (sender, inbox, scheduling::Disabled::new())
    }
}

impl SupervisionConfig for PreparationProbe {
    type Children = supervision::Disabled;

    fn open_children(_: &Self::Options) -> Self::Children {
        supervision::Disabled::new()
    }
}

impl Actor for PreparationProbe {
    type SpawnArgs = usize;

    async fn init(_: usize, _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct PreparationParent;

#[actor(children = 1)]
impl Actor for PreparationParent {
    type SpawnArgs = oneshot::Sender<(usize, bool, usize, usize)>;

    async fn init(result: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let first_opened = Arc::new(AtomicUsize::new(0));
        let first_options = ProbeOptions {
            opened: Arc::clone(&first_opened),
        };
        let Ok(_first) = scope.spawn_child_with::<PreparationProbe>(1, first_options) else {
            panic!("the first child must fit");
        };

        let rejected_opened = Arc::new(AtomicUsize::new(0));
        let rejected_options = ProbeOptions {
            opened: Arc::clone(&rejected_opened),
        };
        let Err(full) = scope.spawn_child_with::<PreparationProbe>(2, rejected_options) else {
            panic!("fixed supervision exceeded its capacity");
        };
        let (args, options) = full.into_inner();
        let same_options = Arc::ptr_eq(&options.opened, &rejected_opened);
        let _ = result.send((
            args,
            same_options,
            first_opened.load(Ordering::SeqCst),
            rejected_opened.load(Ordering::SeqCst),
        ));
        Self
    }
}

// Full admission returns both inputs before child preparation begins.
#[tokio::test(flavor = "current_thread")]
async fn full_spawn_with_returns_options_without_opening_child_config() {
    let (result_tx, result_rx) = oneshot::channel();
    let owner = spawn::<PreparationParent>(result_tx);

    assert_eq!(watchdog(result_rx).await.unwrap(), (2, true, 1, 0));
    let _ = watchdog(owner.shutdown(Shutdown::Stop)).await;
}

struct ExitingChild;

#[actor]
impl Actor for ExitingChild {
    type SpawnArgs = ();

    async fn init(_: (), scope: &mut ActorScope<'_, Self>) -> Self {
        scope.request_shutdown(Shutdown::Stop);
        Self
    }
}

struct RestartingParent {
    replacement: Option<oneshot::Sender<bool>>,
}

#[actor(children = 1)]
impl Actor for RestartingParent {
    type SpawnArgs = oneshot::Sender<bool>;

    async fn init(replacement: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(_child) = scope.spawn_child::<ExitingChild>(()) else {
            panic!("the first child must fit");
        };
        Self {
            replacement: Some(replacement),
        }
    }

    async fn on_child_exit(
        &mut self,
        _event: loong_actor::ChildExit,
        scope: &mut ActorScope<'_, Self>,
    ) {
        let replaced = scope.spawn_child::<IdleChild>(9).is_ok();
        if let Some(replacement) = self.replacement.take() {
            let _ = replacement.send(replaced);
        }
    }
}

// Reaping releases capacity before the child-exit hook starts.
#[tokio::test(flavor = "current_thread")]
async fn child_exit_hook_can_fill_the_reaped_slot() {
    let (replacement_tx, replacement_rx) = oneshot::channel();
    let owner = spawn::<RestartingParent>(replacement_tx);

    assert!(watchdog(replacement_rx).await.unwrap());
    let _ = watchdog(owner.shutdown(Shutdown::Stop)).await;
}
