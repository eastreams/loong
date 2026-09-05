//! Agent-side types for the `loong` product.
//!
//! [`Agent`] is a concrete actor, not a generic family: its context store and
//! provider are type-erased behind [`AgentProvider`] and `Box<dyn ContextStore>`
//! so the rest of the system can hold one actor type.

use std::{collections::VecDeque, future::Future, sync::Arc};

use crate::channel_tool::ChannelTool;
use context::ContextStore;
use contracts::capability::{Capabilities, Capability};
use contracts::provider::{Request, StreamItem};
use contracts::tool::ToolSpec;
use contracts::transcript::{Role, TranscriptItem};
use kernel::policy::action::ActionMeta;
use kernel::{Facade, GrantSendError};
use loac::prelude::*;
use loac::{ActorOwner, ActorRef, Shutdown};
use provider::Provider;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tool_host::{RegisteredTool, RegistrationError, ToolRegistry, ToolSnapshot};

mod builder;
mod channel;
mod channel_tool;
mod tool_set;

/// Agent actor that composes context storage, an upstream provider, a tool
/// host, and an optional system prompt.
pub struct Agent {
    store: Box<dyn ContextStore>,
    provider: AgentProvider,
    registry: ToolRegistry,
    system_prompt: Option<String>,
    /// Prompts that have been accepted by the mailbox but not started yet.
    ///
    /// All reads and writes happen in actor contexts (mailbox handlers and
    /// prompt-future polls), so no lock is needed.
    prompt_queue: VecDeque<QueuedPrompt>,
    /// Cancellation token for the prompt currently running, if any.
    ///
    /// The slot stays `Some` until the running prompt has observed its
    /// cancellation and handed the turn to the next queued prompt, which keeps
    /// new `Prompt` messages queued instead of overlapping the old loop.
    active: Option<CancellationToken>,
    /// Set once by `Actor::on_shutdown`; stops queued prompts from starting
    /// and lets the running one finish.
    draining: bool,
}

impl Agent {
    /// Returns a builder that assembles one agent with explicit resources.
    #[must_use]
    pub fn builder(facade: Facade) -> AgentBuilder {
        AgentBuilder::new(facade)
    }

    /// Starts this agent actor and returns its lifecycle owner.
    pub fn spawn(self) -> ActorOwner<Self> {
        loac::spawn::<Self>(self)
    }

    /// Appends the user message, then snapshots everything the next prompt
    /// loop needs. Runs only in actor contexts, so preparation happens in
    /// mailbox order at the moment the prompt actually starts.
    fn prepare(&mut self, text: String, cancellation: CancellationToken) -> PreparedPrompt {
        let user_item = TranscriptItem::Message {
            role: Role::User,
            text,
            reasoning_content: None,
        };
        let _ = self.store.append(vec![user_item]);

        PreparedPrompt {
            messages: self.store.snapshot().items,
            provider: self.provider.clone(),
            registry: Arc::new(self.registry.snapshot()),
            system_prompt: self.system_prompt.clone(),
            cancellation,
        }
    }

    /// Starts the next queued prompt, if any.
    fn start_next_prompt(&mut self) {
        if self.draining {
            self.active = None;
            return;
        }
        loop {
            let Some(next) = self.prompt_queue.pop_front() else {
                self.active = None;
                return;
            };

            if next.start_tx.is_closed() {
                self.active = None;
                continue;
            }

            let cancellation = CancellationToken::new();
            let prepared = self.prepare(next.text, cancellation.clone());
            if next.start_tx.send(prepared).is_err() {
                self.active = None;
                continue;
            }
            self.active = Some(cancellation);
            return;
        }
    }
}

pub use builder::{AgentBuilder, BuildError};
pub use channel::{ChannelError, ChannelTarget};
pub use tool_set::{FileTools, ToolSet};

/// Type-erased provider used by agents.
pub type AgentProvider = Arc<dyn Provider<Request, StreamItem, ProviderOut>>;

/// Writer that receives streamed provider items.
pub type ProviderOut = mpsc::Sender<StreamItem>;

/// Why a prompt finished without success.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    /// The prompt was cancelled before it could finish, so its transcript
    /// may be incomplete.
    #[error("prompt cancelled")]
    Cancelled,
    /// The provider stream failed.
    #[error("provider stream failed: {0}")]
    Provider(Box<provider::StreamError<Request>>),
}

/// Replaces the provider used by subsequent streams.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct SwitchProvider(pub AgentProvider);

/// Asks the agent to stream a provider reply for one user message.
#[derive(loac::Message)]
#[message(stream = StreamItem, reply = Result<(), PromptError>)]
pub struct Prompt {
    /// The user message to append and send.
    pub text: String,
}

/// Cancels all prompts that are queued but not started yet.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelQueuedPrompts;

/// Cancels the active prompt, if any.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelActivePrompt;

/// Cancels both queued prompts and the active prompt.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelAllPrompts;

/// Registers one named channel as a tool at runtime.
#[derive(loac::Message)]
#[message(reply = Result<(), RegistrationError>)]
pub struct BindChannel {
    pub name: String,
    pub target: Arc<dyn ChannelTarget>,
}

/// Removes one named channel tool.
#[derive(loac::Message)]
#[message(reply = Option<Arc<RegisteredTool>>)]
pub struct UnbindChannel {
    pub name: String,
}

/// The policy action for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SpawnSubagentAction {
    pub name: String,
    pub system_prompt: Option<String>,
    pub tools: Vec<ToolSpec>,
    pub capabilities: Capabilities,
}

impl ActionMeta for SpawnSubagentAction {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("agent.spawn_subagent")
    }

    fn payload(&self) -> std::borrow::Cow<'_, Value> {
        std::borrow::Cow::Owned(serde_json::json!({
            "name": self.name,
            "system_prompt": self.system_prompt,
            "tools": self.tools,
            "capabilities": self.capabilities,
        }))
    }

    fn required_capabilities(&self) -> Capabilities {
        self.capabilities.with(Capability::SpawnSubagent)
    }
}

/// Why spawning a subagent failed.
#[derive(Debug, thiserror::Error)]
pub enum SpawnSubagentError {
    #[error(transparent)]
    Denied(#[from] GrantSendError),
    #[error(transparent)]
    Registration(#[from] RegistrationError),
}

/// Spawns one fully built child agent under the parent runtime.
#[derive(loac::Message)]
#[message(reply = Result<ActorRef<Agent>, SpawnSubagentError>)]
pub struct SpawnSubagent {
    pub name: String,
    pub agent: Agent,
}

/// One-way self-message a finished prompt sends before its final value.
#[derive(loac::Message)]
#[message(reply = ())]
struct PrepareNextPrompt;

/// Owned inputs for one prompt loop.
struct PreparedPrompt {
    messages: Vec<TranscriptItem>,
    provider: AgentProvider,
    registry: Arc<ToolSnapshot>,
    system_prompt: Option<String>,
    cancellation: CancellationToken,
}

/// Queue entry for a prompt whose reply future is waiting for a start signal.
struct QueuedPrompt {
    text: String,
    start_tx: oneshot::Sender<PreparedPrompt>,
}

/// One provider-issued tool call awaiting execution.
#[derive(Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn split_stream_items(items: Vec<StreamItem>) -> (String, String, Vec<PendingToolCall>) {
    let mut reasoning = String::new();
    let mut text = String::new();
    let mut calls = Vec::new();

    for item in items {
        match item {
            StreamItem::ReasoningDelta { delta } => reasoning.push_str(&delta),
            StreamItem::Text { delta } => text.push_str(&delta),
            StreamItem::ToolCall {
                id,
                name,
                arguments,
            } => calls.push(PendingToolCall {
                id,
                name,
                arguments,
            }),
        }
    }

    (reasoning, text, calls)
}

#[actor(mailbox, interleaved = unbounded, children = unbounded)]
impl Actor for Agent {
    type SpawnArgs = Self;

    async fn init(agent: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        agent
    }

    fn on_shutdown(&mut self, _shutdown: Shutdown) {
        self.draining = true;
        // Dropping start senders wakes waiting prompt futures with
        // Cancelled instead of letting them wait forever during Drain.
        self.prompt_queue.clear();
    }
}

impl Handler<SwitchProvider> for Agent {
    async fn handle(message: SwitchProvider, mut cx: Cx<'_, Self>) {
        cx.with(|actor, _| actor.provider = message.0);
    }
}

impl Handler<PrepareNextPrompt> for Agent {
    async fn handle(_message: PrepareNextPrompt, mut cx: Cx<'_, Self>) {
        cx.with(|actor, _| actor.start_next_prompt());
    }
}

impl Handler<CancelQueuedPrompts> for Agent {
    async fn handle(_message: CancelQueuedPrompts, mut cx: Cx<'_, Self>) {
        cx.with(|actor, _| actor.prompt_queue.clear());
    }
}

impl Handler<CancelActivePrompt> for Agent {
    async fn handle(_message: CancelActivePrompt, mut cx: Cx<'_, Self>) {
        cx.with(|actor, _| {
            if let Some(token) = &actor.active {
                token.cancel();
            }
        });
    }
}

impl Handler<CancelAllPrompts> for Agent {
    async fn handle(_message: CancelAllPrompts, mut cx: Cx<'_, Self>) {
        cx.with(|actor, _| {
            actor.prompt_queue.clear();
            if let Some(token) = &actor.active {
                token.cancel();
            }
        });
    }
}

impl Handler<BindChannel> for Agent {
    async fn handle(message: BindChannel, mut cx: Cx<'_, Self>) -> Result<(), RegistrationError> {
        let BindChannel { name, target } = message;
        cx.with(|actor, _| {
            actor
                .registry
                .register(name.clone(), ChannelTool::new(name, target))
        })
    }
}

impl Handler<UnbindChannel> for Agent {
    async fn handle(message: UnbindChannel, mut cx: Cx<'_, Self>) -> Option<Arc<RegisteredTool>> {
        cx.with(|actor, _| actor.registry.unregister(&message.name))
    }
}

impl Handler<SpawnSubagent> for Agent {
    async fn handle(
        message: SpawnSubagent,
        mut cx: Cx<'_, Self>,
    ) -> Result<ActorRef<Agent>, SpawnSubagentError> {
        let SpawnSubagent { name, agent } = message;
        let (facade, action) = cx.with(|actor, _| {
            (
                actor.registry.facade().clone(),
                SpawnSubagentAction {
                    name: name.clone(),
                    system_prompt: agent.system_prompt.clone(),
                    tools: agent.registry.tool_specs(),
                    capabilities: agent.registry.facade().capabilities(),
                },
            )
        });

        let granted = facade.grant(action).await?;
        let (_, action) = granted.into_parts();
        debug_assert_eq!(action.name, name);

        cx.with(|actor, scope| {
            let child = scope
                .spawn_child::<Agent>(agent)
                .unwrap_or_else(|_| unreachable!("unbounded children accept every subagent"));
            let actor_ref = child.actor_ref().clone();
            match actor.registry.register(
                name.clone(),
                ChannelTool::new(name, Arc::new(actor_ref.clone())),
            ) {
                Ok(()) => Ok(actor_ref),
                Err(error) => {
                    actor_ref.request_shutdown(Shutdown::Kill);
                    Err(SpawnSubagentError::Registration(error))
                }
            }
        })
    }
}

impl StreamHandler<Prompt> for Agent {
    fn handle<'a, W>(
        message: Prompt,
        mut out: StreamOut<'a, W>,
        mut cx: Cx<'a, Self>,
    ) -> impl Future<Output = Result<(), PromptError>> + Send + 'a
    where
        W: Writer<StreamItem> + Send + 'a,
    {
        async move {
            let (start_tx, start_rx) = oneshot::channel();
            let myself = cx.with(|actor, scope| {
                actor.prompt_queue.push_back(QueuedPrompt {
                    text: message.text,
                    start_tx,
                });
                if actor.active.is_none() {
                    actor.start_next_prompt();
                }
                scope.myself().clone()
            });

            let prepared = match start_rx.await {
                Ok(prepared) => prepared,
                Err(_) => return Err(PromptError::Cancelled),
            };

            let mut messages = prepared.messages;
            let provider = prepared.provider;
            let registry = prepared.registry;
            let cancellation = prepared.cancellation;

            if let Some(system_prompt) = prepared.system_prompt {
                let has_system = messages.iter().any(|item| {
                    matches!(
                        item,
                        TranscriptItem::Message {
                            role: Role::System,
                            ..
                        }
                    )
                });
                if !has_system {
                    messages.insert(
                        0,
                        TranscriptItem::Message {
                            role: Role::System,
                            text: system_prompt,
                            reasoning_content: None,
                        },
                    );
                }
            }

            let tools = registry.tool_specs();

            loop {
                let request = Request {
                    messages: messages.clone(),
                    tools: tools.clone(),
                };
                let (mut local_tx, mut local_rx) = mpsc::channel::<StreamItem>(8);
                let provider_for_round = provider.clone();

                let mut items = Vec::new();
                let relay = async {
                    while let Some(item) = local_rx.recv().await {
                        let _ = out.write(item.clone()).await;
                        items.push(item);
                    }
                };
                let joined = async {
                    tokio::join!(
                        async move { provider_for_round.stream(request, &mut local_tx).await },
                        relay,
                    )
                };

                let round_result = {
                    let cancellation = cancellation.clone();
                    tokio::select! {
                        biased;
                        result = joined => Some(result),
                        _ = cancellation.cancelled() => None,
                    }
                };

                let (result, ()) = match round_result {
                    Some(joined) => joined,
                    None => {
                        let handed_off = myself.send(PrepareNextPrompt).await.is_ok();
                        if !handed_off {
                            cx.with(|actor, _| actor.prompt_queue.clear());
                        }
                        return Err(PromptError::Cancelled);
                    }
                };

                let (reasoning, text, calls) = split_stream_items(items);
                let reasoning = (!reasoning.is_empty()).then_some(reasoning);

                let mut assistant_items = Vec::new();
                if !text.is_empty() || (calls.is_empty() && reasoning.is_some()) {
                    assistant_items.push(TranscriptItem::Message {
                        role: Role::Assistant,
                        text,
                        reasoning_content: if calls.is_empty() {
                            reasoning.clone()
                        } else {
                            None
                        },
                    });
                }
                for (index, call) in calls.iter().enumerate() {
                    assistant_items.push(TranscriptItem::ToolCall {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        reasoning_content: if index == 0 { reasoning.clone() } else { None },
                    });
                }

                if !assistant_items.is_empty() {
                    let stored = assistant_items.clone();
                    cx.with(|actor, _| {
                        let _ = actor.store.append(stored);
                    });
                    messages.extend(assistant_items);
                }

                if calls.is_empty() {
                    let result = result.map_err(|error| PromptError::Provider(Box::new(error)));
                    let handed_off = myself.send(PrepareNextPrompt).await.is_ok();
                    if !handed_off {
                        cx.with(|actor, _| actor.prompt_queue.clear());
                    }
                    return result;
                }

                let mut pending_calls: VecDeque<PendingToolCall> = calls.into();

                loop {
                    let Some(call) = pending_calls.pop_front() else {
                        break;
                    };

                    let payload = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                    let registry_for_tool = Arc::clone(&registry);
                    let cancellation_for_tool = cancellation.clone();
                    let call_name = call.name.clone();

                    let tool_result = {
                        tokio::select! {
                            biased;
                            result = async move {
                                registry_for_tool.invoke(&call_name, payload).await
                            } => Some(result),
                            _ = cancellation_for_tool.cancelled() => None,
                        }
                    };

                    let result = match tool_result {
                        Some(result) => result,
                        None => {
                            let handed_off = myself.send(PrepareNextPrompt).await.is_ok();
                            if !handed_off {
                                cx.with(|actor, _| actor.prompt_queue.clear());
                            }
                            return Err(PromptError::Cancelled);
                        }
                    };

                    let output = match result {
                        Ok(value) => value.to_string(),
                        Err(error) => format!("tool error: {error}"),
                    };
                    let item = TranscriptItem::ToolResult {
                        call_id: call.id,
                        output,
                    };
                    cx.with(|actor, _| {
                        let _ = actor.store.append(vec![item.clone()]);
                    });
                    messages.push(item);

                    if pending_calls.is_empty() {
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
