use loac::{Actor, ActorScope, CallError, DispatchHandler, Message, ReplyExt, actor};

struct Ponger;

#[actor(mailbox)]
impl Actor for Ponger {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = &'static str)]
struct Ping;

impl DispatchHandler<Ping> for Ponger {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, Ping> + use<> {
        let __reply = { "pong" };
        __reply.ready()
    }
}

struct Pinger {
    ponger: loac::ActorRef<Ponger>,
}

#[actor(mailbox)]
impl Actor for Pinger {
    type SpawnArgs = loac::ActorRef<Ponger>;

    async fn init(ponger: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { ponger }
    }
}

#[derive(Message)]
#[message(reply = String)]
struct AskPong;

impl DispatchHandler<AskPong> for Pinger {
    fn handle(
        &mut self,
        _message: AskPong,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoReply<Self, AskPong> + use<> {
        let ponger = self.ponger.clone();
        async move { ponger.call(Ping).await.unwrap().to_owned() }
    }
}

#[tokio::test]
async fn spawner_hands_out_address_before_spawn() {
    let ponger_spawner = <Ponger as Actor>::spawner();
    let ponger_ref = ponger_spawner.actor_ref();

    let pinger_spawner = <Pinger as Actor>::spawner();
    let pinger_ref = pinger_spawner.actor_ref();

    // Start either actor with the other's preallocated address.
    let _pinger_owner = pinger_spawner.spawn(ponger_ref);
    let _ponger_owner = ponger_spawner.spawn(());

    let reply = pinger_ref.call(AskPong).await.unwrap();
    assert_eq!(reply, "pong");
}

#[tokio::test]
async fn dropping_an_unstarted_spawner_closes_its_address() {
    let spawner = <Ponger as Actor>::spawner();
    let ponger_ref = spawner.actor_ref();
    drop(spawner);

    let error = ponger_ref.call(Ping).await.unwrap_err();
    assert!(matches!(error, CallError::Closed));
}

#[tokio::test]
async fn dropping_an_unstarted_spawner_fails_pre_admitted_calls() {
    use std::time::Duration;

    let spawner = <Ponger as Actor>::spawner();
    let ponger_ref = spawner.actor_ref();

    // Admit a call synchronously, before the spawner is dropped.
    let response = ponger_ref.try_call(Ping).expect("admission should succeed");

    drop(spawner);

    let result = tokio::time::timeout(Duration::from_secs(1), response)
        .await
        .expect("pre-admitted call should be failed by spawner drop, not hang");
    assert!(matches!(
        result,
        Err(CallError::Closed | CallError::BeforeDispatch(_))
    ));
}
