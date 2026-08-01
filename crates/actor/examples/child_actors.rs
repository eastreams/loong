//! Runs a coordinator-owned multi-agent team.
//! Each agent is a child actor.
//! Choose independent roots for independently owned agents.

use loong_actor::{ActorRef, CallError, ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};

#[derive(Debug, PartialEq, Eq)]
struct Report {
    agent: &'static str,
    subject: &'static str,
}

struct Agent(&'static str);

impl Actor for Agent {}

#[derive(Message)]
#[message(reply = Report)]
struct Review(&'static str);

impl SyncHandler<Review> for Agent {
    fn handle(&mut self, message: Review, _scope: &mut ActorScope<Self>) -> Report {
        Report {
            agent: self.0,
            subject: message.0,
        }
    }
}

struct Team {
    agents: Option<[ActorRef<Agent>; 2]>,
}

impl Actor for Team {
    async fn on_start(&mut self, scope: &mut ActorScope<'_, Self>) {
        let correctness = scope.spawn_child(Agent("correctness")).into_actor_ref();
        let readability = scope.spawn_child(Agent("readability")).into_actor_ref();

        // The parent runtime owns both lifecycles. State keeps only addresses.
        self.agents = Some([correctness, readability]);
    }
}

#[derive(Message)]
#[message(reply = Result<[Report; 2], CallError>)]
struct ReviewTask(&'static str);

impl Handler<ReviewTask> for Team {
    fn handle(
        &mut self,
        message: ReviewTask,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, ReviewTask> + use<> {
        let [correctness, readability] = self
            .agents
            .clone()
            .expect("on_start runs before message dispatch");

        async move {
            let (correctness, readability) = tokio::try_join!(
                correctness.call(Review(message.0)),
                readability.call(Review(message.0)),
            )?;
            Ok([correctness, readability])
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Team { agents: None });
    let team = owner.actor_ref();

    let reports = team.call(ReviewTask("actor runtime")).await?;
    let reports = reports?;
    assert_eq!(
        reports,
        [
            Report {
                agent: "correctness",
                subject: "actor runtime",
            },
            Report {
                agent: "readability",
                subject: "actor runtime",
            },
        ]
    );

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
