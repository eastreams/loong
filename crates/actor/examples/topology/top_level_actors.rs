//! Runs independent root actors with separate lifecycle owners.
//! Dropping one owner requests Kill without affecting its peer.

use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};

struct Worker {
    factor: u64,
}

impl Actor for Worker {
    type SpawnArgs = u64;

    async fn init(factor: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { factor }
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Multiply(u64);

impl SyncHandler<Multiply> for Worker {
    fn handle(&mut self, message: Multiply, _scope: &mut ActorScope<Self>) -> u64 {
        self.factor * message.0
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let first_owner = spawn::<Worker>(2);
    let second_owner = spawn::<Worker>(3);
    let first = first_owner.actor_ref();
    let second = second_owner.actor_ref();

    let (first_result, second_result) =
        tokio::join!(first.call(Multiply(5)), second.call(Multiply(5)));
    assert_eq!(first_result?, 10);
    assert_eq!(second_result?, 15);

    drop(first_owner);
    assert_eq!(first.closed().await.reason(), ExitReason::Killed);
    assert_eq!(second.call(Multiply(4)).await?, 12);
    assert_eq!(
        second_owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
    Ok(())
}
