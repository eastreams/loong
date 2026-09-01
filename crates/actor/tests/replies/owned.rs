use super::*;

struct ProgressActor {
    log: Arc<Mutex<Vec<&'static str>>>,
}

#[actor(mailbox, interleaved)]
impl Actor for ProgressActor {
    type SpawnArgs = Arc<Mutex<Vec<&'static str>>>;

    async fn init(log: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { log }
    }
}

impl RawHandler<PendingOwned> for ProgressActor {
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

#[derive(Message)]
#[message(raw = ())]
struct Record(&'static str);

impl RawHandler<Record> for ProgressActor {
    fn handle(
        &mut self,
        message: Record,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Record> + use<> {
        lock(&self.log).push(message.0);
        ().ready()
    }
}

#[derive(Message)]
#[message(raw = ())]
struct InterleavedSequence {
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

#[derive(Message)]
#[message(raw = u8)]
struct ThenSequence;

impl RawHandler<ThenSequence> for ProgressActor {
    fn handle(
        &mut self,
        _message: ThenSequence,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ThenSequence> + use<> {
        async { 1_u8 }
            .into_actor()
            .then(|value, actor: &mut Self, _scope| {
                lock(&actor.log).push("then");
                async move { value + 1 }.into_actor()
            })
            .interleaved()
    }
}

impl RawHandler<InterleavedSequence> for ProgressActor {
    fn handle(
        &mut self,
        message: InterleavedSequence,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, InterleavedSequence> + use<> {
        lock(&self.log).push("interleaved-start");
        let _ = message.started.send(());
        async move {
            let _ = message.release.await;
        }
        .into_actor()
        .map(|(), actor: &mut Self, _scope| {
            lock(&actor.log).push("interleaved-finish");
        })
        .interleaved()
    }
}

#[tokio::test]
async fn owned_does_not_block_mailbox_and_interleaved_reborrows_actor() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let owner = loac::spawn::<ProgressActor>(log.clone());
    let actor = owner.actor_ref();

    let (owned_entered_tx, owned_entered_rx) = oneshot::channel();
    let (owned_release_tx, owned_release_rx) = oneshot::channel();
    let owned = actor
        .try_call(PendingOwned {
            entered: owned_entered_tx,
            release: owned_release_rx,
        })
        .unwrap();
    watchdog(owned_entered_rx).await.unwrap();
    assert_eq!(watchdog(actor.call(Record("during-owned"))).await, Ok(()));

    let (interleaved_started_tx, interleaved_started_rx) = oneshot::channel();
    let (interleaved_release_tx, interleaved_release_rx) = oneshot::channel();
    let interleaved = actor
        .try_call(InterleavedSequence {
            started: interleaved_started_tx,
            release: interleaved_release_rx,
        })
        .unwrap();
    watchdog(interleaved_started_rx).await.unwrap();
    assert_eq!(watchdog(actor.call(Record("between-polls"))).await, Ok(()));

    owned_release_tx.send(()).unwrap();
    interleaved_release_tx.send(()).unwrap();
    assert_eq!(watchdog(owned).await, Ok(()));
    assert_eq!(watchdog(interleaved).await, Ok(()));
    assert_eq!(watchdog(actor.call(ThenSequence)).await, Ok(2));
    assert_eq!(
        *lock(&log),
        vec![
            "during-owned",
            "interleaved-start",
            "between-polls",
            "interleaved-finish",
            "then"
        ]
    );

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
