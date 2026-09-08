use loac::{Actor, ActorScope, Cx, ExitReason, Handler, Message, Shutdown, actor};

struct Worker;

#[actor(mailbox, interleaved = unbounded)]
impl Actor for Worker {
    type SpawnArgs = ();

    async fn init((): (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = ())]
struct Ping;

impl Handler<Ping> for Worker {
    async fn handle(_message: Ping, cx: Cx<'_, Self>) {
        // `Cx::myself` returns the same address the scope would provide.
        assert!(cx.myself().exit_status().is_none());
        // `Deref` makes ActorRef methods callable directly on `cx`.
        assert!(cx.exit_status().is_none());
    }
}

#[tokio::test]
async fn cx_exposes_the_actor_ref() {
    let owner = loac::spawn::<Worker>(());

    owner.call(Ping).await.expect("ping should be handled");

    let exit = owner.shutdown(Shutdown::Stop).await;
    assert_eq!(exit.reason(), ExitReason::Stopped);
}
