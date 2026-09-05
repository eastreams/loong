use loac::{Actor, ActorScope, Message, SyncHandler, actor, spawn, sync_handler};

struct Counter(u64);

#[actor(mailbox)]
impl Actor for Counter {
    type SpawnArgs = u64;

    async fn init(value: u64, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

#[sync_handler]
impl SyncHandler<Add> for Counter {
    fn handle(&mut self, message: Add, _scope: &mut ActorScope<Self>) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[derive(Message)]
#[message(reply = ())]
struct Reset;

#[sync_handler]
impl SyncHandler<Reset> for Counter {
    fn handle(&mut self, _message: Reset, _scope: &mut ActorScope<Self>) {
        self.0 = 0;
    }
}

#[tokio::test]
async fn sync_handler_dispatches_without_interleaving() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn::<Counter>(1);

    assert_eq!(owner.call(Add(2)).await?, 3);
    owner.send(Reset).await?;
    assert_eq!(owner.call(Add(4)).await?, 4);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}

struct ConstService<const N: usize>;

#[actor(mailbox = dynamic(N))]
impl<const N: usize> Actor for ConstService<N> {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = usize)]
struct ReadN;

#[sync_handler]
impl<const N: usize> SyncHandler<ReadN> for ConstService<N> {
    fn handle(&mut self, _message: ReadN, _scope: &mut ActorScope<Self>) -> usize {
        N
    }
}

#[tokio::test]
async fn sync_handler_preserves_impl_generics() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn::<ConstService<8>>(());
    assert_eq!(owner.call(ReadN).await?, 8);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}
