//! Uses one root actor to own two child workers.
//! Choose this topology when one coordinator owns worker lifetimes.
//! The coordinator's bare future uses owned reply scheduling.

use loong_actor::{ActorRef, CallError, ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};

struct Worker(u64);

impl Actor for Worker {}

#[derive(Message)]
#[message(reply = u64)]
struct Multiply(u64);

impl SyncHandler<Multiply> for Worker {
    fn handle(&mut self, message: Multiply, _scope: &mut ActorScope<Self>) -> u64 {
        self.0 * message.0
    }
}

struct Coordinator {
    workers: Option<[ActorRef<Worker>; 2]>,
}

impl Actor for Coordinator {
    async fn on_start(&mut self, scope: &mut ActorScope<Self>) {
        let double = scope
            .spawn_child(Worker(2))
            .expect("on_start accepts children")
            .into_actor_ref();
        let triple = scope
            .spawn_child(Worker(3))
            .expect("on_start accepts children")
            .into_actor_ref();

        // ActorScope owns both lifecycles. State retains only message addresses.
        self.workers = Some([double, triple]);
    }
}

#[derive(Message)]
#[message(reply = Result<[u64; 2], CallError>)]
struct Compute(u64);

impl Handler<Compute> for Coordinator {
    fn handle(
        &mut self,
        message: Compute,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Compute> + use<> {
        let [double, triple] = self
            .workers
            .clone()
            .expect("on_start runs before message dispatch");

        async move {
            let (doubled, tripled) = tokio::try_join!(
                double.call(Multiply(message.0)),
                triple.call(Multiply(message.0)),
            )?;
            Ok([doubled, tripled])
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Coordinator { workers: None });
    let coordinator = owner.actor_ref();

    let worker_results = coordinator.call(Compute(7)).await?;
    assert_eq!(worker_results?, [14, 21]);

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
