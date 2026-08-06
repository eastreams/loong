//! Runs a coordinator-owned multi-agent team.
//! Each agent is a child actor.
//! Choose independent roots for independently owned agents.

use loac::{ActorRef, CallError, ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};

#[derive(Debug, PartialEq, Eq)]
struct Report {
    agent: &'static str,
    subject: &'static str,
}

struct Agent(&'static str);

#[actor(mailbox)]
impl Actor for Agent {
    type SpawnArgs = &'static str;

    async fn init(name: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(name)
    }
}

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
    agents: [ActorRef<Agent>; 2],
}

#[actor(mailbox, children = unbounded)]
impl Actor for Team {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let Ok(correctness) = scope.spawn_child::<Agent>("correctness");
        let Ok(readability) = scope.spawn_child::<Agent>("readability");
        let correctness = correctness.into_actor_ref();
        let readability = readability.into_actor_ref();

        // The parent runtime owns both lifecycles. State keeps only addresses.
        Self {
            agents: [correctness, readability],
        }
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
        let [correctness, readability] = self.agents.clone();

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
    let owner = spawn::<Team>(());
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
