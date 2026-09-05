//! Planner / reviewer / workers workflow actor.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use agent::{
    Agent, BindChannel, BuildError, ChannelTarget, Prompt, PromptError, SpawnSubagent,
    SpawnSubagentError,
};
use config::{AgentConfig, ConfigError, ProviderConfig, StoreConfig, ToolConfig};
use contracts::capability::{Capabilities, Capability};
use contracts::provider::StreamItem;
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
use loac::prelude::*;
use loac::{ActorOwner, ActorRef, Shutdown};
use provider_openai::OpenAiConfig;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Cli,
    input::{StdinUserInput, UserCommand},
    print_stream_item,
};

/// The workflow actor.
pub struct Workflow {
    reviewer_ref: ActorRef<Agent>,
    planner_ref: ActorRef<Agent>,
    provider: ProviderConfig,
    empty_facade: Facade,
    max_workers: usize,
    max_review_rounds: usize,
    max_llm_retries: usize,
}

impl Workflow {
    #[must_use]
    pub fn builder(
        facade: Facade,
        provider: ProviderConfig,
        workspace_root: impl Into<PathBuf>,
    ) -> WorkflowBuilder {
        WorkflowBuilder::new(facade, provider, workspace_root)
    }
}

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
    #[error("agent config failed: {0}")]
    Config(#[from] ConfigError),
    #[error("actor call failed: {0}")]
    Call(#[from] loac::CallError),
    #[error("prompt failed: {0}")]
    Prompt(#[from] PromptError),
    #[error("planner did not produce a valid plan after retries")]
    PlanRetryExhausted,
    #[error("reviewer did not produce valid review JSON after retries")]
    ReviewRetryExhausted,
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

/// Asks the workflow to run one goal.
#[derive(loac::Message)]
#[message(stream = StreamItem, reply = Result<(), WorkflowError>)]
pub struct RunGoal {
    pub goal: String,
}

/// Workflow assembly options.
pub struct WorkflowBuilder {
    root_facade: Facade,
    empty_facade: Facade,
    provider: ProviderConfig,
    workspace_root: PathBuf,
    max_workers: usize,
    max_review_rounds: usize,
    max_llm_retries: usize,
}

impl WorkflowBuilder {
    #[must_use]
    pub fn new(
        facade: Facade,
        provider: ProviderConfig,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        let empty_facade = facade.clone().narrow(Capabilities::empty());
        Self {
            root_facade: facade,
            empty_facade,
            provider,
            workspace_root: workspace_root.into(),
            max_workers: 8,
            max_review_rounds: 3,
            max_llm_retries: 3,
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

    #[must_use]
    pub fn with_max_llm_retries(mut self, max_llm_retries: usize) -> Self {
        self.max_llm_retries = max_llm_retries;
        self
    }

    pub fn spawn(self) -> ActorOwner<Workflow> {
        loac::spawn::<Workflow>(self)
    }
}

#[actor(mailbox, interleaved = unbounded, children = unbounded)]
impl Actor for Workflow {
    type SpawnArgs = WorkflowBuilder;

    async fn init(builder: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let reviewer_caps = Capabilities::empty()
            .with(Capability::FsRead)
            .with(Capability::FsWrite)
            .with(Capability::SpawnSubagent);
        let reviewer_facade = builder.root_facade.clone().narrow(reviewer_caps);
        let no_channels: HashMap<String, Arc<dyn ChannelTarget>> = HashMap::new();
        let reviewer_agent = AgentConfig {
            name: "reviewer".into(),
            system_prompt: Some(REVIEWER_SYSTEM_PROMPT.into()),
            workspace_root: builder.workspace_root.clone(),
            capabilities: reviewer_caps,
            store: StoreConfig::Memory,
            provider: builder.provider.clone(),
            tools: vec![ToolConfig::FileTools],
            channels: vec![],
        }
        .build(reviewer_facade, &no_channels)
        .expect("reviewer agent is valid");
        let reviewer = scope
            .spawn_child::<Agent>(reviewer_agent)
            .unwrap_or_else(|_| unreachable!("unbounded children accept reviewer"));
        let reviewer_ref = reviewer.actor_ref().clone();

        let mut channels: HashMap<String, Arc<dyn ChannelTarget>> = HashMap::new();
        channels.insert("reviewer".into(), Arc::new(reviewer_ref.clone()));

        let planner_agent = AgentConfig {
            name: "planner".into(),
            system_prompt: Some(PLANNER_SYSTEM_PROMPT.into()),
            workspace_root: builder.workspace_root.clone(),
            capabilities: Capabilities::empty(),
            store: StoreConfig::Memory,
            provider: builder.provider.clone(),
            tools: vec![],
            channels: vec!["reviewer".into()],
        }
        .build(builder.empty_facade.clone(), &channels)
        .expect("planner agent is valid");
        let planner = scope
            .spawn_child::<Agent>(planner_agent)
            .unwrap_or_else(|_| unreachable!("unbounded children accept planner"));
        let planner_ref = planner.actor_ref().clone();

        Self {
            reviewer_ref,
            planner_ref,
            provider: builder.provider,
            empty_facade: builder.empty_facade,
            max_workers: builder.max_workers,
            max_review_rounds: builder.max_review_rounds,
            max_llm_retries: builder.max_llm_retries,
        }
    }
}

impl StreamHandler<RunGoal> for Workflow {
    fn handle<'a, W>(
        message: RunGoal,
        mut out: StreamOut<'a, W>,
        mut cx: Cx<'a, Self>,
    ) -> impl Future<Output = Result<(), WorkflowError>> + Send + 'a
    where
        W: Writer<StreamItem> + Send + 'a,
    {
        let (
            planner_ref,
            reviewer_ref,
            provider,
            empty_facade,
            max_workers,
            max_review_rounds,
            max_llm_retries,
        ) = cx.with(|actor, _| {
            (
                actor.planner_ref.clone(),
                actor.reviewer_ref.clone(),
                actor.provider.clone(),
                actor.empty_facade.clone(),
                actor.max_workers,
                actor.max_review_rounds,
                actor.max_llm_retries,
            )
        });

        async move {
            let result = run_workflow(
                &planner_ref,
                &reviewer_ref,
                &provider,
                &empty_facade,
                max_workers,
                max_review_rounds,
                max_llm_retries,
                message.goal,
            )
            .await;

            match result {
                Ok(answer) => {
                    let _ = out.write(StreamItem::Text { delta: answer }).await;
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_workflow(
    planner_ref: &ActorRef<Agent>,
    reviewer_ref: &ActorRef<Agent>,
    provider: &ProviderConfig,
    empty_facade: &Facade,
    max_workers: usize,
    max_review_rounds: usize,
    max_llm_retries: usize,
    request: String,
) -> Result<String, WorkflowError> {
    let plan = plan(planner_ref, request, max_workers, max_llm_retries).await?;
    let plan = review_plan(
        planner_ref,
        reviewer_ref,
        plan,
        max_workers,
        max_review_rounds,
        max_llm_retries,
    )
    .await?;
    spawn_workers(reviewer_ref, planner_ref, provider, empty_facade, &plan).await?;

    let tasks_json = serde_json::to_string(&plan.tasks).expect("worker tasks serialize as JSON");
    let execute_prompt = format!(
        "Execute the following tasks using the worker_i tools, then synthesize a final answer.\nTasks:\n{tasks_json}"
    );
    let final_answer = ask_agent(planner_ref, execute_prompt, max_llm_retries).await?;

    review_final(
        planner_ref,
        reviewer_ref,
        final_answer,
        max_review_rounds,
        max_llm_retries,
    )
    .await
}

async fn plan(
    planner_ref: &ActorRef<Agent>,
    request: String,
    max_workers: usize,
    retries: usize,
) -> Result<Plan, WorkflowError> {
    let prompt = format!(
        "Task:\n{request}\n\nProduce an execution plan. Respond ONLY with JSON, no markdown:\n\
         {{\"workers\": <n>, \"tasks\": [{{\"worker\": <i>, \"prompt\": \"...\"}}]}}\n\
         workers must be between 1 and {}.",
        max_workers
    );
    ask_for_plan(planner_ref, prompt, max_workers, retries).await
}

async fn review_plan(
    planner_ref: &ActorRef<Agent>,
    reviewer_ref: &ActorRef<Agent>,
    mut plan: Plan,
    max_workers: usize,
    max_review_rounds: usize,
    retries: usize,
) -> Result<Plan, WorkflowError> {
    let mut approved = false;
    for _ in 0..max_review_rounds {
        let review_prompt = format!(
            "Review this plan JSON:\n{}\n\nRespond ONLY with JSON: \
             {{\"approved\": true}} or {{\"approved\": false, \"feedback\": \"...\"}}.",
            serde_json::to_string(&plan).expect("plan serializes as JSON")
        );
        let review = ask_for_review(reviewer_ref, review_prompt, retries).await?;
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
        plan = ask_for_plan(planner_ref, revise_prompt, max_workers, retries).await?;
    }

    if approved {
        Ok(plan)
    } else {
        Err(WorkflowError::PlanReviewExhausted)
    }
}

async fn spawn_workers(
    reviewer_ref: &ActorRef<Agent>,
    planner_ref: &ActorRef<Agent>,
    provider: &ProviderConfig,
    empty_facade: &Facade,
    plan: &Plan,
) -> Result<(), WorkflowError> {
    for index in 0..plan.workers {
        let name = format!("worker_{index}");
        let mut channels: HashMap<String, Arc<dyn ChannelTarget>> = HashMap::new();
        channels.insert("reviewer".into(), Arc::new(reviewer_ref.clone()));
        let worker_agent = AgentConfig {
            name: name.clone(),
            system_prompt: Some(worker_system_prompt(index)),
            workspace_root: PathBuf::from("."),
            capabilities: Capabilities::empty(),
            store: StoreConfig::Memory,
            provider: provider.clone(),
            tools: vec![],
            channels: vec!["reviewer".into()],
        }
        .build(empty_facade.clone(), &channels)?;

        let worker_ref = reviewer_ref
            .call(SpawnSubagent {
                name: name.clone(),
                agent: worker_agent,
            })
            .await??;

        planner_ref
            .call(BindChannel {
                name,
                target: Arc::new(worker_ref),
            })
            .await??;
    }
    Ok(())
}

async fn review_final(
    planner_ref: &ActorRef<Agent>,
    reviewer_ref: &ActorRef<Agent>,
    mut answer: String,
    max_review_rounds: usize,
    retries: usize,
) -> Result<String, WorkflowError> {
    for _ in 0..max_review_rounds {
        let review_prompt = format!(
            "Review this final answer:\n{answer}\n\nRespond ONLY with JSON: \
             {{\"approved\": true}} or {{\"approved\": false, \"feedback\": \"...\"}}."
        );
        let review = ask_for_review(reviewer_ref, review_prompt, retries).await?;
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
        answer = ask_agent(planner_ref, revise_prompt, retries).await?;
    }

    Err(WorkflowError::FinalReviewExhausted)
}

async fn ask_agent(
    target: &ActorRef<Agent>,
    text: String,
    retries: usize,
) -> Result<String, WorkflowError> {
    let mut prompt = text;
    for attempt in 0..=retries {
        let mut reply = target
            .call(Prompt {
                text: prompt.clone(),
            })
            .await?;
        let mut answer = String::new();
        while let Some(item) = reply.recv().await {
            if let StreamItem::Text { delta } = item {
                answer.push_str(&delta);
            }
        }
        match reply.finish().await? {
            Ok(()) => return Ok(answer),
            Err(PromptError::Provider(error)) => {
                if attempt == retries {
                    return Err(PromptError::Provider(error).into());
                }
                prompt = format!(
                    "{prompt}\n\n(The previous attempt failed with an upstream provider error: {error}; please try again.)"
                );
            }
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("ask_agent retry loop always returns")
}

async fn ask_for_plan(
    target: &ActorRef<Agent>,
    mut prompt: String,
    max_workers: usize,
    retries: usize,
) -> Result<Plan, WorkflowError> {
    for _ in 0..=retries {
        let text = ask_agent(target, prompt.clone(), retries).await?;
        match parse_json::<Plan>(&text) {
            Ok(plan) => match validate_plan(&plan, max_workers) {
                Ok(()) => return Ok(plan),
                Err(error) => {
                    prompt = format!(
                        "Your plan was invalid: {error}\nRespond ONLY with JSON in the same schema."
                    );
                }
            },
            Err(error) => {
                prompt = format!(
                    "Your plan JSON was invalid: {error}\nRespond ONLY with JSON in the same schema."
                );
            }
        }
    }
    Err(WorkflowError::PlanRetryExhausted)
}

async fn ask_for_review(
    target: &ActorRef<Agent>,
    mut prompt: String,
    retries: usize,
) -> Result<PlanReview, WorkflowError> {
    for _ in 0..=retries {
        let text = ask_agent(target, prompt.clone(), retries).await?;
        match parse_json::<PlanReview>(&text) {
            Ok(review) => return Ok(review),
            Err(error) => {
                prompt = format!(
                    "Your review JSON was invalid: {error}\nRespond ONLY with JSON: \
                     {{\"approved\": true}} or {{\"approved\": false, \"feedback\": \"...\"}}."
                );
            }
        }
    }
    Err(WorkflowError::ReviewRetryExhausted)
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

pub(crate) async fn run(
    cli: Cli,
    max_workers: usize,
    max_review_rounds: usize,
    max_llm_retries: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = cli.model.clone();

    let kernel = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());

    let mut input = StdinUserInput::new(tokio::io::stdin());

    while let Some(command) = input.next().await? {
        match command {
            UserCommand::Quit => break,
            UserCommand::SwitchModel(new_model) => {
                model = new_model.clone();
                println!("switched to {new_model}");
            }
            UserCommand::Prompt(text) => {
                let facade = Facade::new(
                    kernel.actor_ref(),
                    [
                        Capability::FsRead,
                        Capability::FsWrite,
                        Capability::SpawnSubagent,
                    ],
                );
                let provider = ProviderConfig::OpenAi(OpenAiConfig::new(
                    cli.base_url.clone(),
                    cli.api_key.clone(),
                    model.clone(),
                ));

                let workflow = Workflow::builder(facade, provider, &cli.workspace)
                    .with_max_workers(max_workers)
                    .with_max_review_rounds(max_review_rounds)
                    .with_max_llm_retries(max_llm_retries);
                let owner = workflow.spawn();

                let mut reply = match owner.call(RunGoal { goal: text }).await {
                    Ok(reply) => reply,
                    Err(error) => {
                        eprintln!("workflow error: {error}");
                        let _ = owner.shutdown(Shutdown::Drain).await;
                        continue;
                    }
                };

                while let Some(item) = reply.recv().await {
                    print_stream_item(item);
                }
                match reply.finish().await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => eprintln!("workflow error: {error}"),
                    Err(error) => eprintln!("workflow error: {error}"),
                }
                println!();
                let _ = owner.shutdown(Shutdown::Drain).await;
            }
        }
    }

    let _ = kernel.shutdown(Shutdown::Drain).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use agent::ProviderOut;
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

        let owner =
            Workflow::builder(facade, ProviderConfig::Resolved(Arc::new(provider)), ".").spawn();
        let mut reply = owner
            .call(RunGoal {
                goal: "do it".to_string(),
            })
            .await
            .unwrap();
        let mut answer = String::new();
        while let Some(item) = reply.recv().await {
            if let StreamItem::Text { delta } = item {
                answer.push_str(&delta);
            }
        }
        reply.finish().await.unwrap().unwrap();
        assert_eq!(answer, "final answer");

        let status = owner.shutdown(Shutdown::Drain).await;
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
