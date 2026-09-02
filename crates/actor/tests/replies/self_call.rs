use super::*;

struct SelfCaller;

#[actor(mailbox = 8, interleaved = dynamic)]
impl Actor for SelfCaller {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct Echo(u8);

impl RawHandler<Echo> for SelfCaller {
    fn handle(
        &mut self,
        message: Echo,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Echo> + use<> {
        message.0.ready()
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct OwnedSelfCall(u8);

impl RawHandler<OwnedSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: OwnedSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, OwnedSelfCall> + use<> {
        let response = scope.try_call(Echo(message.0)).unwrap();
        async move { response.await.unwrap() }
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct InterleavedSelfCall(u8);

impl RawHandler<InterleavedSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: InterleavedSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, InterleavedSelfCall> + use<> {
        let response = scope.try_call(Echo(message.0)).unwrap();
        async move { response.await.unwrap() }
            .into_actor()
            .interleaved()
    }
}

#[derive(Message)]
#[message(raw = ())]
struct ExclusiveSelfCall {
    observed: oneshot::Sender<Response<u8>>,
    polled: oneshot::Sender<()>,
}

impl RawHandler<ExclusiveSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: ExclusiveSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ExclusiveSelfCall> + use<> {
        let awaited = scope.try_call(Echo(1)).unwrap();
        let observed = scope.try_call(Echo(2)).unwrap();
        let _ = message.observed.send(observed);
        async move {
            let _ = message.polled.send(());
            awaited.await.unwrap();
        }
        .into_actor()
        .exclusive()
    }
}

#[tokio::test]
async fn nonexclusive_self_calls_progress_but_exclusive_self_call_waits() {
    let options =
        SpawnOptions::<SelfCaller>::default().with_max_in_flight(NonZeroUsize::new(4).unwrap());
    let mut owner = spawn_with::<SelfCaller>((), options);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(OwnedSelfCall(3))).await, Ok(3));
    assert_eq!(watchdog(actor.call(InterleavedSelfCall(4))).await, Ok(4));

    let (observed_tx, observed_rx) = oneshot::channel();
    let (polled_tx, polled_rx) = oneshot::channel();
    let outer = actor
        .try_call(ExclusiveSelfCall {
            observed: observed_tx,
            polled: polled_tx,
        })
        .unwrap();
    let observed = watchdog(observed_rx).await.unwrap();
    watchdog(polled_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        loac::ShutdownStatus::Requested
    );
    assert_eq!(
        watchdog(outer).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(
        watchdog(observed).await,
        Err(CallError::BeforeDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Killed);
}

#[tokio::test]
async fn owned_self_call_progresses_with_one_interleaved_slot() {
    // Owned tasks consume no interleaved slot.
    // The inner call can use the configured slot.
    let options =
        SpawnOptions::<SelfCaller>::default().with_max_in_flight(NonZeroUsize::new(1).unwrap());
    let owner = spawn_with::<SelfCaller>((), options);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(OwnedSelfCall(1))).await, Ok(1));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
