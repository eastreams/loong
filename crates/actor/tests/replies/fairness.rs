use super::*;

struct FairActor {
    handled: Arc<AtomicUsize>,
}

#[actor(mailbox = 64, interleaved)]
impl Actor for FairActor {
    type SpawnArgs = Arc<AtomicUsize>;

    async fn init(handled: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { handled }
    }
}

#[derive(Message)]
#[message(raw = bool)]
struct OwnedTaskIdentity;

impl RawHandler<OwnedTaskIdentity> for FairActor {
    fn handle(
        &mut self,
        _message: OwnedTaskIdentity,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, OwnedTaskIdentity> + use<> {
        let actor_task = tokio::task::id();
        async move { tokio::task::id() != actor_task }
    }
}

#[derive(Message)]
#[message(raw = ())]
struct ActiveInterleavedReply {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    completed_at: Arc<AtomicUsize>,
}

impl RawHandler<ActiveInterleavedReply> for FairActor {
    fn handle(
        &mut self,
        message: ActiveInterleavedReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ActiveInterleavedReply> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            message.completed_at
        }
        .into_actor()
        .map(|completed_at, actor: &mut Self, _scope| {
            completed_at.store(actor.handled.load(Ordering::SeqCst), Ordering::SeqCst);
        })
        .interleaved()
    }
}

#[derive(Message)]
#[message(raw = ())]
struct ReadyWork;

impl RawHandler<ReadyWork> for FairActor {
    fn handle(
        &mut self,
        _message: ReadyWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ReadyWork> + use<> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        ().ready()
    }
}

// Task identity distinguishes spawning from actor-local polling.
// Progress alone would pass under both implementations.
#[tokio::test]
async fn owned_reply_runs_in_a_distinct_tokio_task() {
    let handled = Arc::new(AtomicUsize::new(0));
    let owner = loac::spawn::<FairActor>(handled.clone());
    let actor = owner.actor_ref();
    assert_eq!(watchdog(actor.call(OwnedTaskIdentity)).await, Ok(true));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}

#[tokio::test]
async fn ready_mailbox_input_does_not_starve_woken_interleaved_reply() {
    let handled = Arc::new(AtomicUsize::new(0));
    let owner = loac::spawn::<FairActor>(handled.clone());
    let actor = owner.actor_ref();
    let completed_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let active = actor
        .try_call(ActiveInterleavedReply {
            entered: entered_tx,
            release: release_rx,
            completed_at: completed_at.clone(),
        })
        .unwrap();
    tokio::pin!(active);
    watchdog(entered_rx).await.unwrap();
    assert!(poll_once(active.as_mut()).await.is_pending());

    let queued: Vec<_> = (0..32)
        .map(|_| actor.try_call(ReadyWork).unwrap())
        .collect();
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(active).await, Ok(()));
    assert!(completed_at.load(Ordering::SeqCst) < queued.len());

    for response in queued {
        assert_eq!(watchdog(response).await, Ok(()));
    }
    assert_eq!(handled.load(Ordering::SeqCst), 32);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}

struct FairChildExitArgs {
    child_started: oneshot::Sender<ActorRef<HookChild>>,
    handled: Arc<AtomicUsize>,
    hook_completed_at: Arc<AtomicUsize>,
    hook_completed: oneshot::Sender<()>,
}

struct FairChildExitActor {
    handled: Arc<AtomicUsize>,
    hook_completed_at: Arc<AtomicUsize>,
    hook_completed: Option<oneshot::Sender<()>>,
}

#[actor(mailbox = 64, children = unbounded)]
impl Actor for FairChildExitActor {
    type SpawnArgs = FairChildExitArgs;

    async fn init(args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(child) = scope.spawn_child::<HookChild>(());
        let child = child.into_actor_ref();
        let _ = args.child_started.send(child);
        Self {
            handled: args.handled,
            hook_completed_at: args.hook_completed_at,
            hook_completed: Some(args.hook_completed),
        }
    }

    async fn on_child_exit(&mut self, _event: ChildExit, _scope: &mut ActorScope<'_, Self>) {
        self.hook_completed_at
            .store(self.handled.load(Ordering::SeqCst), Ordering::SeqCst);
        if let Some(completed) = self.hook_completed.take() {
            let _ = completed.send(());
        }
    }
}

impl RawHandler<PendingOwned> for FairChildExitActor {
    fn handle(
        &mut self,
        message: PendingOwned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, PendingOwned> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
    }
}

impl RawHandler<ExclusiveGate> for FairChildExitActor {
    fn handle(
        &mut self,
        message: ExclusiveGate,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ExclusiveGate> + use<> {
        async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        }
        .into_actor()
        .exclusive()
    }
}

impl RawHandler<ReadyWork> for FairChildExitActor {
    fn handle(
        &mut self,
        _message: ReadyWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ReadyWork> + use<> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        ().ready()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn queued_child_exit_progresses_before_ready_mailbox_is_exhausted() {
    let handled = Arc::new(AtomicUsize::new(0));
    let hook_completed_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (child_started_tx, child_started_rx) = oneshot::channel();
    let (hook_completed_tx, hook_completed_rx) = oneshot::channel();
    let owner = loac::spawn::<FairChildExitActor>(FairChildExitArgs {
        child_started: child_started_tx,
        handled: handled.clone(),
        hook_completed_at: hook_completed_at.clone(),
        hook_completed: hook_completed_tx,
    });
    let actor = owner.actor_ref();
    let child = watchdog(child_started_rx).await.unwrap();

    let (owned_entered_tx, owned_entered_rx) = oneshot::channel();
    let (owned_release_tx, owned_release_rx) = oneshot::channel();
    let owned = actor
        .try_call(PendingOwned {
            entered: owned_entered_tx,
            release: owned_release_rx,
        })
        .unwrap();
    watchdog(owned_entered_rx).await.unwrap();

    let (exclusive_entered_tx, exclusive_entered_rx) = oneshot::channel();
    let (exclusive_release_tx, exclusive_release_rx) = oneshot::channel();
    let exclusive = actor
        .try_call(ExclusiveGate {
            entered: exclusive_entered_tx,
            release: exclusive_release_rx,
        })
        .unwrap();
    watchdog(exclusive_entered_rx).await.unwrap();

    assert_eq!(watchdog(child.call(StopChild)).await, Ok(()));
    // On the current-thread runtime the child publishes its supervisor event
    // before this task resumes; exclusive keeps the parent from consuming it.
    assert_eq!(watchdog(child.closed()).await.reason(), ExitReason::Stopped);
    let mut hook_completed = Box::pin(hook_completed_rx);
    assert!(poll_once(hook_completed.as_mut()).await.is_pending());

    let queued: Vec<_> = (0..32)
        .map(|_| actor.try_call(ReadyWork).unwrap())
        .collect();
    exclusive_release_tx.send(()).unwrap();
    assert_eq!(watchdog(exclusive).await, Ok(()));
    watchdog(hook_completed).await.unwrap();
    assert!(hook_completed_at.load(Ordering::SeqCst) < queued.len());

    owned_release_tx.send(()).unwrap();
    assert_eq!(watchdog(owned).await, Ok(()));
    for response in queued {
        assert_eq!(watchdog(response).await, Ok(()));
    }
    assert_eq!(handled.load(Ordering::SeqCst), 32);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
