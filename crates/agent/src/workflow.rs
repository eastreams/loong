//! Planner / reviewer / workers workflow.
//!
//! The reviewer is the root agent and the only capability holder. It owns the
//! planner and every worker as child actors. The planner and workers are
//! sandboxed (empty capability ceiling); any file side effect must go through
//! the reviewer's channel.
//!
//! Worker count is not fixed at assembly time. A `WorkflowHandle::run` first
//! asks the planner for a plan, has the reviewer approve or revise it, and
//! only then spawns the number of workers the approved plan requires. Each
//! worker is bound into the planner as a `worker_i` channel tool before the
//! execution prompt starts.

use std::path::PathBuf;
use std::sync::Arc;

use context::memory::MemoryStore;
use contracts::capability::Capabilities;
use contracts::provider::{Request, StreamItem};
use kernel::Facade;
use loac::{ActorOwner, ActorRef, Shutdown};
use provider::Provider;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Agent, BindChannel, BuildError, FileTools, Prompt, PromptError, ProviderOut, SpawnSubagent,
    SpawnSubagentError,
};

const REVIEWER_SYSTEM_PROMPT: &str = "\
You are the reviewer and the only agent that owns file capabilities and file tools. \
When asked to review a plan or a final answer, respond ONLY with JSON, no markdown: \
{\"approved\": true} or {\"approved\": false, \"feedback\": \"...\"}. \
When asked to perform file operations, use read_file or write_file and report the result.";

const PLANNER_SYSTEM_PROMPT: &str = "\
You are the planner in a planner-reviewer-workers workflow. \
When asked for a plan, respond ONLY with JSON, no markdown, in the schema: \
{\"workers\": <n>, \"tasks\": [{\"worker\": <i>, \"prompt\": \"...\"}]}. \
When asked to execute tasks, use the worker_i tools to delegate work, then synthesize a final answer.";

fn worker_system_prompt(index: usize) -> String {
    format!(
        "You are worker {index} in a planner-reviewer-workers workflow. \
         You have no file tools. If your task needs file operations, call the \
         `reviewer` tool with a clear instruction and wait for its result. \
         Execute your assigned task and return a concise result."
    )
}

/// One planner task assigned to one worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerTask {
    pub worker: usize,
    pub prompt: String,
}

/// The planner's plan: how many workers to spawn and which tasks to dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub workers: usize,
    pub tasks: Vec<WorkerTask>,
}

/// A structured reviewer decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanReview {
    approved: bool,
    #[serde(default)]
    feedback: Option<String>,
}

/// Why workflow assembly or execution failed.
#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("agent build failed: {0}")]
    Build(#[from] BuildError),
    #[error("actor call failed: {0}")]
    Call(#[from] loac::CallError),
    #[error("prompt failed: {0}")]
    Prompt(#[from] PromptError),
    #[error("plan parse failed: {0}")]
    PlanParse(#[source] serde_json::Error),
    #[error("review parse failed: {0}")]
    ReviewParse(#[source] serde_json::Error),
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
    #[error("spawn worker failed: {0}")]
    Spawn(#[from] SpawnSubagentError),
    #[error("bind worker channel failed: {0}")]
    Bind(#[from] tool_host::RegistrationError),
    #[error("plan review did not converge")]
    PlanReviewExhausted,
    #[error("final review did not converge")]
    FinalReviewExhausted,
}

/// Workflow assembly options.
pub struct Workflow<P> {
    facade: Facade,
    provider: P,
    workspace_root: PathBuf,
    max_workers: usize,
    max_review_rounds: usize,
}

impl<P> Workflow<P> {
    #[must_use]
    pub fn new(facade: Facade, provider: P, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            facade,
            provider,
            workspace_root: workspace_root.into(),
            max_workers: 8,
            max_review_rounds: 3,
        }
    }

    #[must_use]
    pub fn with_max_workers(mut self, max_workers: usize) -> Self {
        self.max_workers = max_workers;
        self
    }

    #[must_use]
    pub fn with_max_review_rounds(mut self, max_review_rounds: usize) -> Self {
        self.max_review_rounds = max_review_rounds;
        self
    }

    /// Spawns the reviewer root and the planner child. Workers are spawned
    /// later by [`WorkflowHandle::run`], after the plan is known.
    pub async fn spawn(self) -> Result<WorkflowHandle<P>, WorkflowError>
    where
        P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    {
        let reviewer_agent = Agent::builder(self.facade.clone())
            .with(FileTools)
            .with_workspace_root(&self.workspace_root)
            .with_system_prompt(REVIEWER_SYSTEM_PROMPT)
            .with_store(MemoryStore::new())
            .with_provider(self.provider.clone())
            .build()?;

        let reviewer_owner = reviewer_agent.spawn();
        let reviewer_ref = reviewer_owner.actor_ref();

        let empty_facade = self.facade.clone().narrow(Capabilities::empty());
        let planner_agent = Agent::builder(empty_facade.clone())
            .with_channel("reviewer", Arc::new(reviewer_ref.clone()))
            .with_system_prompt(PLANNER_SYSTEM_PROMPT)
            .with_store(MemoryStore::new())
            .with_provider(self.provider.clone())
            .build()?;

        let planner_ref = reviewer_ref
            .call(SpawnSubagent {
                name: "planner".to_string(),
                agent: planner_agent,
            })
            .await??;

        Ok(WorkflowHandle {
            reviewer_owner,
            reviewer_ref,
            planner_ref,
            provider: self.provider,
            empty_facade,
            max_workers: self.max_workers,
            max_review_rounds: self.max_review_rounds,
        })
    }
}

/// A running workflow.
pub struct WorkflowHandle<P>
where
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    reviewer_owner: ActorOwner<Agent<MemoryStore, P>>,
    reviewer_ref: ActorRef<Agent<MemoryStore, P>>,
    planner_ref: ActorRef<Agent<MemoryStore, P>>,
    provider: P,
    empty_facade: Facade,
    max_workers: usize,
    max_review_rounds: usize,
}

impl<P> WorkflowHandle<P>
where
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    #[must_use]
    pub fn reviewer_ref(&self) -> &ActorRef<Agent<MemoryStore, P>> {
        &self.reviewer_ref
    }

    #[must_use]
    pub fn planner_ref(&self) -> &ActorRef<Agent<MemoryStore, P>> {
        &self.planner_ref
    }

    /// Runs the workflow once for one user request.
    ///
    /// This is intentionally single-shot: workers are spawned from the plan
    /// and bound into the planner during this call. Run a fresh workflow for a
    /// new request.
    pub async fn run(&self, request: String) -> Result<String, WorkflowError> {
        let plan = self.plan(request).await?;
        let plan = self.review_plan(plan).await?;
        self.spawn_workers(&plan).await?;

        let tasks_json =
            serde_json::to_string(&plan.tasks).expect("worker tasks serialize as JSON");
        let execute_prompt = format!(
            "Execute the following tasks using the worker_i tools, then synthesize a final answer.\nTasks:\n{tasks_json}"
        );
        let final_answer = ask_agent(&self.planner_ref, execute_prompt).await?;

        self.review_final(final_answer).await
    }

    /// Shuts down the reviewer root; the runtime takes planner and workers
    /// with it.
    pub async fn shutdown(self) -> loac::ExitStatus {
        self.reviewer_owner.shutdown(Shutdown::Drain).await
    }

    async fn plan(&self, request: String) -> Result<Plan, WorkflowError> {
        let prompt = format!(
            "Task:\n{request}\n\nProduce an execution plan. Respond ONLY with JSON, no markdown:\n\
             {{\"workers\": <n>, \"tasks\": [{{\"worker\": <i>, \"prompt\": \"...\"}}]}}\n\
             workers must be between 1 and {}.",
            self.max_workers
        );
        let text = ask_agent(&self.planner_ref, prompt).await?;
        let plan = parse_json::<Plan>(&text).map_err(WorkflowError::PlanParse)?;
        validate_plan(&plan, self.max_workers)?;
        Ok(plan)
    }

    async fn review_plan(&self, mut plan: Plan) -> Result<Plan, WorkflowError> {
        let mut approved = false;
        for _ in 0..self.max_review_rounds {
            let review_prompt = format!(
                "Review this plan JSON:\n{}\n\nRespond ONLY with JSON: \
                 {{\"approved\": true}} or {{\"approved\": false, \"feedback\": \"...\"}}.",
                serde_json::to_string(&plan).expect("plan serializes as JSON")
            );
            let review_text = ask_agent(&self.reviewer_ref, review_prompt).await?;
            let review =
                parse_json::<PlanReview>(&review_text).map_err(WorkflowError::ReviewParse)?;
            if review.approved {
                approved = true;
                break;
            }

            let feedback = review
                .feedback
                .unwrap_or_else(|| "No feedback provided.".to_string());
            let revise_prompt = format!(
                "Your plan was rejected with this feedback:\n{feedback}\n\nOriginal plan:\n{}\n\n\
                 Revise the plan and respond ONLY with JSON in the same schema.",
                serde_json::to_string(&plan).expect("plan serializes as JSON")
            );
            let revised_text = ask_agent(&self.planner_ref, revise_prompt).await?;
            plan = parse_json::<Plan>(&revised_text).map_err(WorkflowError::PlanParse)?;
            validate_plan(&plan, self.max_workers)?;
        }

        if approved {
            Ok(plan)
        } else {
            Err(WorkflowError::PlanReviewExhausted)
        }
    }

    async fn spawn_workers(&self, plan: &Plan) -> Result<(), WorkflowError> {
        for index in 0..plan.workers {
            let name = format!("worker_{index}");
            let worker_agent = Agent::builder(self.empty_facade.clone())
                .with_channel("reviewer", Arc::new(self.reviewer_ref.clone()))
                .with_system_prompt(worker_system_prompt(index))
                .with_store(MemoryStore::new())
                .with_provider(self.provider.clone())
                .build()?;

            let worker_ref = self
                .reviewer_ref
                .call(SpawnSubagent {
                    name: name.clone(),
                    agent: worker_agent,
                })
                .await??;

            self.planner_ref
                .call(BindChannel {
                    name,
                    target: Arc::new(worker_ref),
                })
                .await??;
        }
        Ok(())
    }

    async fn review_final(&self, mut answer: String) -> Result<String, WorkflowError> {
        for _ in 0..self.max_review_rounds {
            let review_prompt = format!(
                "Review this final answer:\n{answer}\n\nRespond ONLY with JSON: \
                 {{\"approved\": true}} or {{\"approved\": false, \"feedback\": \"...\"}}."
            );
            let review_text = ask_agent(&self.reviewer_ref, review_prompt).await?;
            let review =
                parse_json::<PlanReview>(&review_text).map_err(WorkflowError::ReviewParse)?;
            if review.approved {
                return Ok(answer);
            }

            let feedback = review
                .feedback
                .unwrap_or_else(|| "No feedback provided.".to_string());
            let revise_prompt = format!(
                "Your final answer was rejected with this feedback:\n{feedback}\n\nPrevious answer:\n{answer}\n\n\
                 Revise the final answer and respond with plain text."
            );
            answer = ask_agent(&self.planner_ref, revise_prompt).await?;
        }

        Err(WorkflowError::FinalReviewExhausted)
    }
}

async fn ask_agent<P>(
    target: &ActorRef<Agent<MemoryStore, P>>,
    text: String,
) -> Result<String, WorkflowError>
where
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    let mut reply = target.call(Prompt { text }).await?;
    let mut answer = String::new();
    while let Some(item) = reply.recv().await {
        if let StreamItem::Text { delta } = item {
            answer.push_str(&delta);
        }
    }
    reply.finish().await??;
    Ok(answer)
}

fn validate_plan(plan: &Plan, max_workers: usize) -> Result<(), WorkflowError> {
    if plan.workers == 0 || plan.workers > max_workers {
        return Err(WorkflowError::InvalidPlan(format!(
            "workers must be between 1 and {max_workers}, got {}",
            plan.workers
        )));
    }
    if plan.tasks.is_empty() {
        return Err(WorkflowError::InvalidPlan(
            "tasks must not be empty".to_string(),
        ));
    }
    for task in &plan.tasks {
        if task.worker >= plan.workers {
            return Err(WorkflowError::InvalidPlan(format!(
                "task references worker {} but workers is {}",
                task.worker, plan.workers
            )));
        }
        if task.prompt.trim().is_empty() {
            return Err(WorkflowError::InvalidPlan(
                "task prompt must not be empty".to_string(),
            ));
        }
    }
    Ok(())
}

fn parse_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {
    let trimmed = text.trim();
    if let Some(inner) = trimmed
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
    {
        return serde_json::from_str(inner.trim());
    }
    if let Some(inner) = trimmed
        .strip_prefix("```")
        .and_then(|s| s.strip_suffix("```"))
    {
        return serde_json::from_str(inner.trim());
    }
    serde_json::from_str(trimmed)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use contracts::capability::{Capabilities, Capability};
    use contracts::provider::{Request, StreamItem};
    use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
    use loac::Shutdown;
    use provider::{Provider, StreamError};

    use super::*;

    #[derive(Clone)]
    struct ScriptedProvider {
        responses: Arc<Mutex<VecDeque<String>>>,
    }

    #[async_trait]
    impl Provider<Request, StreamItem, ProviderOut> for ScriptedProvider {
        async fn stream(
            &self,
            _req: Request,
            out: &mut ProviderOut,
        ) -> Result<(), StreamError<Request>> {
            let text = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default();
            out.send(StreamItem::Text { delta: text }).await.unwrap();
            Ok(())
        }
    }

    fn workflow_facade() -> (loac::ActorOwner<Kernel>, Facade) {
        let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
        let capabilities = Capabilities::empty()
            .with(Capability::FsRead)
            .with(Capability::FsWrite)
            .with(Capability::SpawnSubagent);
        let facade = Facade::new(kernel_owner.actor_ref(), capabilities);
        (kernel_owner, facade)
    }

    #[tokio::test]
    async fn workflow_runs_plan_review_execute_review_sequence() {
        let (kernel_owner, facade) = workflow_facade();
        let provider = ScriptedProvider {
            responses: Arc::new(Mutex::new(VecDeque::from([
                "{\"workers\":1,\"tasks\":[{\"worker\":0,\"prompt\":\"do it\"}]}".to_string(),
                "{\"approved\":true}".to_string(),
                "final answer".to_string(),
                "{\"approved\":true}".to_string(),
            ]))),
        };

        let handle = Workflow::new(facade, provider, ".").spawn().await.unwrap();
        let answer = handle.run("do it".to_string()).await.unwrap();
        assert_eq!(answer, "final answer");

        let status = handle.shutdown().await;
        assert_eq!(status.reason(), loac::ExitReason::Drained);
        let _ = kernel_owner.shutdown(Shutdown::Drain).await;
    }

    #[test]
    fn plan_validation_rejects_invalid_plans() {
        let valid = Plan {
            workers: 2,
            tasks: vec![WorkerTask {
                worker: 0,
                prompt: "a".to_string(),
            }],
        };
        assert!(validate_plan(&valid, 2).is_ok());

        let no_workers = Plan {
            workers: 0,
            tasks: vec![WorkerTask {
                worker: 0,
                prompt: "a".to_string(),
            }],
        };
        assert!(validate_plan(&no_workers, 2).is_err());

        let too_many_workers = Plan {
            workers: 3,
            tasks: vec![WorkerTask {
                worker: 0,
                prompt: "a".to_string(),
            }],
        };
        assert!(validate_plan(&too_many_workers, 2).is_err());

        let no_tasks = Plan {
            workers: 2,
            tasks: vec![],
        };
        assert!(validate_plan(&no_tasks, 2).is_err());

        let bad_worker = Plan {
            workers: 2,
            tasks: vec![WorkerTask {
                worker: 2,
                prompt: "a".to_string(),
            }],
        };
        assert!(validate_plan(&bad_worker, 2).is_err());

        let empty_prompt = Plan {
            workers: 2,
            tasks: vec![WorkerTask {
                worker: 0,
                prompt: " ".to_string(),
            }],
        };
        assert!(validate_plan(&empty_prompt, 2).is_err());
    }
}
