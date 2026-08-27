//! Agent-side types for the `loong` product.

use std::{marker::PhantomData, path::PathBuf, sync::Arc};

use context::ContextStore;
use contracts::capability::{Capabilities, Capability};
use contracts::provider::{Request, StreamItem};
use contracts::transcript::{Role, TranscriptItem};
use loac::prelude::*;
use provider::{Provider, StreamError};
use serde_json::Value;
use tokio::sync::mpsc;
use tool_host::{InvocationParams, ToolRegistry};

/// Writer that receives streamed provider items.
pub type ProviderOut = mpsc::Sender<StreamItem>;

/// A typed agent role.
///
/// Profiles own the tool set, capability envelope, and system prompt for one
/// kind of agent. Spawn [`Agent<C, P, K>`](Agent) with the profile as the last
/// spawn argument, or use a type alias such as [`FileIoAgent`].
pub trait AgentProfile: Send + Sync + 'static {
    /// Stable profile name used in spans and errors.
    fn name(&self) -> &'static str;

    /// Capability ceiling for this agent kind.
    fn capabilities(&self) -> Capabilities {
        Capabilities::empty()
    }

    /// System prompt prepended to provider requests when the transcript does
    /// not already start with a system message.
    fn system_prompt(&self) -> Option<String> {
        None
    }

    /// Registers the tools this agent kind may call.
    fn register_tools(&self, _registry: &mut ToolRegistry) {}
}

/// File I/O agent profile: read and write files inside the workspace.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileIoProfile;

impl AgentProfile for FileIoProfile {
    fn name(&self) -> &'static str {
        "file_io"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::empty()
            .with(Capability::FsRead)
            .with(Capability::FsWrite)
    }

    fn system_prompt(&self) -> Option<String> {
        Some(
            "You are a file I/O agent. Use read_file and write_file for workspace files."
                .to_owned(),
        )
    }

    fn register_tools(&self, registry: &mut ToolRegistry) {
        let _ = registry.register("read_file".to_owned(), tools::ReadFileTool);
        let _ = registry.register("write_file".to_owned(), tools::WriteFileTool);
    }
}

/// Planning agent profile: no tools, planning system prompt.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlanProfile;

impl AgentProfile for PlanProfile {
    fn name(&self) -> &'static str {
        "plan"
    }

    fn system_prompt(&self) -> Option<String> {
        Some("You are a planning agent. Produce concise, ordered plans.".to_owned())
    }
}

/// Supervisor agent profile placeholder.
///
/// Delegation to worker agents lands later; this profile currently only owns a
/// supervisor system prompt.
#[derive(Debug, Clone, Copy, Default)]
pub struct SupervisorProfile;

impl AgentProfile for SupervisorProfile {
    fn name(&self) -> &'static str {
        "supervisor"
    }

    fn system_prompt(&self) -> Option<String> {
        Some("You are a supervisor agent. Coordinate workers and tools.".to_owned())
    }
}

/// File I/O agent type alias.
pub type FileIoAgent<C, P> = Agent<C, P, FileIoProfile>;
/// Planning agent type alias.
pub type PlanAgent<C, P> = Agent<C, P, PlanProfile>;
/// Supervisor agent type alias.
pub type SupervisorAgent<C, P> = Agent<C, P, SupervisorProfile>;

/// Agent actor that composes context storage, an upstream provider, a tool
/// host, a workspace root, and a role profile.
///
/// `P: Clone` keeps stream handlers able to capture the provider they were
/// started with, so switching the actor's provider never interrupts streams
/// that are already running.
pub struct Agent<C, P, K>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut> + Clone,
    K: AgentProfile,
{
    store: C,
    provider: P,
    registry: Arc<ToolRegistry>,
    workspace_root: PathBuf,
    system_prompt: Option<String>,
    _profile: PhantomData<fn() -> K>,
}

/// Replaces the provider used by subsequent streams.
///
/// The frontend builds a new provider (for example an OpenAI provider with a
/// different model) and sends it to the agent. Streams that already started
/// hold their own clone, so they keep their original provider.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct SwitchProvider<P>(pub P);

/// Asks the agent to stream a provider reply for one user message.
///
/// The agent appends the user message to its context store, then runs the
/// provider/tool loop until the model returns text without tool calls.
#[derive(loac::Message)]
#[message(stream = StreamItem, reply = Result<(), StreamError<Request>>)]
pub struct Prompt {
    /// The user message to append and send.
    pub text: String,
}

/// Appends transcript items to the store from an owned stream task.
///
/// This is a private self-message so the tool loop can commit partial
/// transcripts without borrowing actor state across awaits.
#[derive(loac::Message)]
#[message(reply = ())]
struct CommitTranscript(Vec<TranscriptItem>);

#[actor(mailbox)]
impl<C, P, K> Actor for Agent<C, P, K>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    K: AgentProfile,
{
    type SpawnArgs = (C, P, ToolRegistry, PathBuf, K);

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        let (store, provider, registry, workspace_root, profile) = args;
        let system_prompt = profile.system_prompt();

        Self {
            store,
            provider,
            registry: Arc::new(registry),
            workspace_root,
            system_prompt,
            _profile: PhantomData,
        }
    }
}

impl<C, P, K> SyncHandler<SwitchProvider<P>> for Agent<C, P, K>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    K: AgentProfile,
{
    fn handle(&mut self, message: SwitchProvider<P>, _scope: &mut ActorScope<'_, Self>) {
        self.provider = message.0;
    }
}

impl<C, P, K> SyncHandler<CommitTranscript> for Agent<C, P, K>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    K: AgentProfile,
{
    fn handle(&mut self, message: CommitTranscript, _scope: &mut ActorScope<'_, Self>) {
        let _ = self.store.append(message.0);
    }
}

/// One provider-issued tool call awaiting execution.
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
}

fn split_stream_items(items: Vec<StreamItem>) -> (String, Vec<PendingToolCall>) {
    let mut text = String::new();
    let mut calls = Vec::new();

    for item in items {
        match item {
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

    (text, calls)
}

impl<C, P, K> StreamHandler<Prompt> for Agent<C, P, K>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    K: AgentProfile,
{
    fn handle<W>(
        &mut self,
        message: Prompt,
        mut out: W,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, Prompt> + use<C, P, K, W>
    where
        W: loac::Writer<StreamItem> + Send + 'static,
    {
        let provider = self.provider.clone();
        let registry = Arc::clone(&self.registry);
        let workspace_root = self.workspace_root.clone();
        let system_prompt = self.system_prompt.clone();
        let myself = scope.myself().clone();

        let user_item = TranscriptItem::Message {
            role: Role::User,
            text: message.text,
        };
        let _ = self.store.append(vec![user_item]);
        let snapshot = self.store.snapshot().items;

        async move {
            let mut messages = snapshot;

            if let Some(system_prompt) = system_prompt {
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
                let provider = provider.clone();

                let relay = async {
                    let mut items = Vec::new();
                    while let Some(item) = local_rx.recv().await {
                        let _ = out.write(item.clone()).await;
                        items.push(item);
                    }
                    items
                };

                let (result, items) = tokio::join!(
                    async move { provider.stream(request, &mut local_tx).await },
                    relay,
                );

                let (text, calls) = split_stream_items(items);

                let mut assistant_items = Vec::new();
                if !text.is_empty() {
                    assistant_items.push(TranscriptItem::Message {
                        role: Role::Assistant,
                        text,
                    });
                }
                for call in &calls {
                    assistant_items.push(TranscriptItem::ToolCall {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    });
                }

                if !assistant_items.is_empty() {
                    let _ = myself.call(CommitTranscript(assistant_items.clone())).await;
                    messages.extend(assistant_items);
                }

                if calls.is_empty() {
                    return result;
                }

                for call in calls {
                    let payload = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
                    let params = InvocationParams::new(&workspace_root);
                    let output = match registry.invoke(&call.name, &params, payload).await {
                        Ok(value) => value.to_string(),
                        Err(error) => format!("tool error: {error}"),
                    };
                    let item = TranscriptItem::ToolResult {
                        call_id: call.id,
                        output,
                    };
                    let _ = myself.call(CommitTranscript(vec![item.clone()])).await;
                    messages.push(item);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
