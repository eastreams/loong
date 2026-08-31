//! Agent-side types for the `loong` product.
//!
//! [`Agent`] is a concrete actor, not a generic family: its context store and
//! provider are type-erased behind [`AgentProvider`] and `Box<dyn ContextStore>`
//! so the rest of the system can hold one actor type.

use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

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
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;
use tool_host::{RegisteredTool, RegistrationError, ToolError, ToolRegistry, ToolSnapshot};

mod builder;
mod channel;
mod channel_tool;
mod tool_set;

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
    /// [`AgentLoop`] polls), so no lock is needed.
    prompt_queue: VecDeque<QueuedPrompt>,
    /// Cancellation token for the prompt currently running, if any.
    ///
    /// The slot stays `Some` until the running prompt has observed its
    /// cancellation and handed the turn to the next queued prompt, which keeps
    /// new `Prompt` messages queued instead of overlapping the old loop.
    active: Option<CancellationToken>,
    /// The next turn, prepared by [`Agent::start_next_prompt`] and waiting for
    /// [`AgentLoop`] to poll it.
    current_turn: Option<QueuedTurn>,
    /// Bumped at every turn end; waiters use it for `this_turn.await`.
    turn_epoch: u64,
    turn_watch: watch::Sender<u64>,
    /// Set once by [`Actor::on_shutdown`]; makes [`AgentLoop`] exit when it
    /// runs out of queued work.
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

    /// Starts the next queued prompt, if any and if no turn is active.
    ///
    /// Called from the mailbox (`PrepareNextPrompt`, first `Prompt`) so the
    /// start decision is FIFO-ordered with every steer message admitted before
    /// it.
    fn start_next_prompt(&mut self) {
        if self.draining || self.active.is_some() || self.current_turn.is_some() {
            return;
        }
        let Some(queued) = self.prompt_queue.pop_front() else {
            return;
        };

        let cancellation = CancellationToken::new();
        let prepared = self.prepare(queued.text, cancellation.clone());
        self.active = Some(cancellation);
        self.current_turn = Some(QueuedTurn {
            prepared,
            item_tx: queued.item_tx,
            final_tx: queued.final_tx,
        });
    }

    /// Marks one turn as finished: bump the epoch so `this_turn.await` waiters
    /// wake up.
    fn finish_turn(&mut self) {
        self.turn_epoch += 1;
        self.turn_watch.send_replace(self.turn_epoch);
    }
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

/// One queued prompt waiting for the fixed loop.
struct QueuedPrompt {
    text: String,
    item_tx: mpsc::Sender<StreamItem>,
    final_tx: oneshot::Sender<Result<(), PromptError>>,
}

/// A turn that has been prepared by the mailbox and is waiting for
/// [`AgentLoop`] to poll it.
struct QueuedTurn {
    prepared: PreparedPrompt,
    item_tx: mpsc::Sender<StreamItem>,
    final_tx: oneshot::Sender<Result<(), PromptError>>,
}

/// Owned inputs for one turn.
struct PreparedPrompt {
    messages: Vec<TranscriptItem>,
    provider: AgentProvider,
    registry: Arc<ToolSnapshot>,
    system_prompt: Option<String>,
    cancellation: CancellationToken,
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

/// All mutable turn state, moved through the stage futures.
struct LoopData<W> {
    messages: Vec<TranscriptItem>,
    provider: AgentProvider,
    registry: Arc<ToolSnapshot>,
    tools: Vec<ToolSpec>,
    cancellation: CancellationToken,
    pending_calls: VecDeque<PendingToolCall>,
    out: W,
}

impl<W> LoopData<W> {
    fn from_prepared(prepared: PreparedPrompt, out: W) -> Self {
        let PreparedPrompt {
            mut messages,
            provider,
            registry,
            system_prompt,
            cancellation,
        } = prepared;

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
                        reasoning_content: None,
                    },
                );
            }
        }

        let tools = registry.tool_specs();

        Self {
            messages,
            provider,
            registry,
            tools,
            cancellation,
            pending_calls: VecDeque::new(),
            out,
        }
    }
}

enum RoundOutcome<W> {
    Completed {
        data: Box<LoopData<W>>,
        result: Result<(), provider::StreamError<Request>>,
        items: Vec<StreamItem>,
    },
    Cancelled,
}

enum ToolOutcome<W> {
    Completed {
        data: Box<LoopData<W>>,
        result: Result<Value, ToolError>,
    },
    Cancelled,
}

type RoundFuture<W> = Pin<Box<dyn Future<Output = RoundOutcome<W>> + Send>>;
type ToolFuture<W> = Pin<Box<dyn Future<Output = ToolOutcome<W>> + Send>>;

fn build_round_future<W>(data: LoopData<W>) -> RoundFuture<W>
where
    W: Writer<StreamItem> + Send + 'static,
{
    let LoopData {
        messages,
        provider,
        registry,
        tools,
        cancellation,
        pending_calls: _,
        mut out,
    } = data;

    let request = Request {
        messages: messages.clone(),
        tools: tools.clone(),
    };
    let (mut local_tx, mut local_rx) = mpsc::channel::<StreamItem>(8);
    let provider_for_round = provider.clone();

    Box::pin(async move {
        let relay = async move {
            let mut items = Vec::new();
            while let Some(item) = local_rx.recv().await {
                let _ = out.write(item.clone()).await;
                items.push(item);
            }
            (out, items)
        };

        tokio::select! {
            biased;
            joined = async {
                tokio::join!(
                    async move { provider_for_round.stream(request, &mut local_tx).await },
                    relay,
                )
            } => {
                let (result, (out, items)) = joined;
                RoundOutcome::Completed {
                    data: Box::new(LoopData {
                        messages,
                        provider,
                        registry,
                        tools,
                        cancellation,
                        pending_calls: VecDeque::new(),
                        out,
                    }),
                    result,
                    items,
                }
            }
            _ = cancellation.cancelled() => RoundOutcome::Cancelled,
        }
    })
}

fn build_tool_future<W>(data: LoopData<W>, call: PendingToolCall) -> ToolFuture<W>
where
    W: Writer<StreamItem> + Send + 'static,
{
    let payload = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
    let registry = Arc::clone(&data.registry);
    let cancellation = data.cancellation.clone();
    let call_name = call.name.clone();

    Box::pin(async move {
        tokio::select! {
            biased;
            result = async { registry.invoke(&call_name, payload).await } => {
                ToolOutcome::Completed { data: Box::new(data), result }
            }
            _ = cancellation.cancelled() => ToolOutcome::Cancelled,
        }
    })
}

type HandoffFuture = Pin<Box<dyn Future<Output = bool> + Send>>;

enum AgentLoopStage {
    Idle,
    Round {
        future: RoundFuture<mpsc::Sender<StreamItem>>,
        final_tx: oneshot::Sender<Result<(), PromptError>>,
    },
    Tool {
        future: ToolFuture<mpsc::Sender<StreamItem>>,
        call: PendingToolCall,
        final_tx: oneshot::Sender<Result<(), PromptError>>,
    },
    Handoff {
        future: HandoffFuture,
    },
}

/// The fixed prompt-loop driver.
struct AgentLoop {
    stage: AgentLoopStage,
    myself: ActorRef<Agent>,
}

impl AgentLoop {
    fn new(myself: ActorRef<Agent>) -> Self {
        Self {
            stage: AgentLoopStage::Idle,
            myself,
        }
    }

    fn build_handoff_future(&self) -> HandoffFuture {
        let myself = self.myself.clone();
        Box::pin(async move { myself.send(PrepareNextPrompt).await.is_ok() })
    }
}

/// Starts the fixed [`AgentLoop`].
#[derive(loac::Message)]
#[message(reply = ())]
struct EnsureLoop;

#[actor(mailbox, interleaved = unbounded, children = unbounded)]
impl Actor for Agent {
    type SpawnArgs = Self;

    async fn init(agent: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let myself = scope.myself().clone();
        myself
            .send(EnsureLoop)
            .await
            .expect("self admission is open");
        agent
    }

    fn on_shutdown(&mut self, _shutdown: Shutdown) {
        self.draining = true;
    }
}

impl Handler<EnsureLoop> for Agent {
    fn handle(
        &mut self,
        _message: EnsureLoop,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, EnsureLoop> + use<> {
        AgentLoop::new(scope.myself().clone()).interleaved()
    }
}

/// Mailbox-ordered boundary marker sent after every finished turn.
#[derive(loac::Message)]
#[message(reply = ())]
struct PrepareNextPrompt;

impl SyncHandler<PrepareNextPrompt> for Agent {
    fn handle(&mut self, _message: PrepareNextPrompt, _scope: &mut ActorScope<'_, Self>) {
        self.start_next_prompt();
    }
}

impl SyncHandler<SwitchProvider> for Agent {
    fn handle(&mut self, message: SwitchProvider, _scope: &mut ActorScope<'_, Self>) {
        self.provider = message.0;
    }
}

impl SyncHandler<CancelQueuedPrompts> for Agent {
    fn handle(&mut self, _message: CancelQueuedPrompts, _scope: &mut ActorScope<'_, Self>) {
        self.prompt_queue.clear();
    }
}

impl SyncHandler<CancelActivePrompt> for Agent {
    fn handle(&mut self, _message: CancelActivePrompt, _scope: &mut ActorScope<'_, Self>) {
        if let Some(token) = &self.active {
            token.cancel();
        }
    }
}

impl SyncHandler<CancelAllPrompts> for Agent {
    fn handle(&mut self, _message: CancelAllPrompts, _scope: &mut ActorScope<'_, Self>) {
        self.prompt_queue.clear();
        if let Some(token) = &self.active {
            token.cancel();
        }
    }
}

impl SyncHandler<BindChannel> for Agent {
    fn handle(
        &mut self,
        message: BindChannel,
        _scope: &mut ActorScope<'_, Self>,
    ) -> Result<(), RegistrationError> {
        let BindChannel { name, target } = message;
        self.registry
            .register(name.clone(), ChannelTool::new(name, target))
    }
}

impl SyncHandler<UnbindChannel> for Agent {
    fn handle(
        &mut self,
        message: UnbindChannel,
        _scope: &mut ActorScope<'_, Self>,
    ) -> Option<Arc<RegisteredTool>> {
        self.registry.unregister(&message.name)
    }
}

impl Handler<SpawnSubagent> for Agent {
    fn handle(
        &mut self,
        message: SpawnSubagent,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, SpawnSubagent> + use<> {
        let facade = self.registry.facade().clone();
        let SpawnSubagent { name, agent } = message;
        let action = SpawnSubagentAction {
            name: name.clone(),
            system_prompt: agent.system_prompt.clone(),
            tools: agent.registry.tool_specs(),
            capabilities: agent.registry.facade().capabilities(),
        };

        async move { facade.grant(action).await }
            .into_actor()
            .then(
                move |granted, actor: &mut Agent, scope: &mut ActorScope<'_, Agent>| {
                    let result = match granted {
                        Ok(granted) => {
                            let (_, action) = granted.into_parts();
                            debug_assert_eq!(action.name, name);
                            let child = scope.spawn_child::<Agent>(agent).unwrap_or_else(|_| {
                                unreachable!("unbounded children accept every subagent")
                            });
                            let actor_ref = child.actor_ref().clone();
                            match actor.registry.register(
                                name.clone(),
                                ChannelTool::new(name.clone(), Arc::new(actor_ref.clone())),
                            ) {
                                Ok(()) => Ok(actor_ref),
                                Err(error) => {
                                    actor_ref.request_shutdown(Shutdown::Kill);
                                    Err(SpawnSubagentError::Registration(error))
                                }
                            }
                        }
                        Err(error) => Err(SpawnSubagentError::Denied(error)),
                    };
                    std::future::ready(result).into_actor()
                },
            )
            .interleaved()
    }
}

impl StreamHandler<Prompt> for Agent {
    fn handle<W>(
        &mut self,
        message: Prompt,
        out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoStreamReply<Self, Prompt> + use<W>
    where
        W: Writer<StreamItem> + Send + 'static,
    {
        let (item_tx, mut item_rx) = mpsc::channel::<StreamItem>(8);
        let (final_tx, final_rx) = oneshot::channel();

        self.prompt_queue.push_back(QueuedPrompt {
            text: message.text,
            item_tx,
            final_tx,
        });

        if self.active.is_none() && self.current_turn.is_none() {
            self.start_next_prompt();
        }

        async move {
            let mut out = out;
            while let Some(item) = item_rx.recv().await {
                let _ = out.write(item).await;
            }
            match final_rx.await {
                Ok(result) => result,
                Err(_) => Err(PromptError::Cancelled),
            }
        }
    }
}

impl ActorFuture<Agent> for AgentLoop {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut Agent,
        _scope: &mut ActorScope<'_, Agent>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        let this = self.get_mut();

        loop {
            let mut stage = AgentLoopStage::Idle;
            std::mem::swap(&mut this.stage, &mut stage);

            match stage {
                AgentLoopStage::Idle => {
                    if actor.draining {
                        if let Some(turn) = actor.current_turn.take() {
                            let _ = turn.final_tx.send(Err(PromptError::Cancelled));
                        }
                        while let Some(queued) = actor.prompt_queue.pop_front() {
                            let _ = queued.final_tx.send(Err(PromptError::Cancelled));
                        }
                        actor.active = None;
                        return Poll::Ready(());
                    }
                    if let Some(turn) = actor.current_turn.take() {
                        let data = LoopData::from_prepared(turn.prepared, turn.item_tx);
                        this.stage = AgentLoopStage::Round {
                            future: build_round_future(data),
                            final_tx: turn.final_tx,
                        };
                    } else {
                        actor.active = None;
                        return Poll::Pending;
                    }
                }
                AgentLoopStage::Round {
                    mut future,
                    final_tx,
                } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = AgentLoopStage::Round { future, final_tx };
                        return Poll::Pending;
                    }
                    Poll::Ready(RoundOutcome::Cancelled) => {
                        let _ = final_tx.send(Err(PromptError::Cancelled));
                        actor.active = None;
                        actor.finish_turn();
                        this.stage = AgentLoopStage::Handoff {
                            future: this.build_handoff_future(),
                        };
                    }
                    Poll::Ready(RoundOutcome::Completed {
                        mut data,
                        result,
                        items,
                    }) => {
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
                                reasoning_content: if index == 0 {
                                    reasoning.clone()
                                } else {
                                    None
                                },
                            });
                        }

                        if !assistant_items.is_empty() {
                            let _ = actor.store.append(assistant_items.clone());
                            data.messages.extend(assistant_items);
                        }

                        if calls.is_empty() {
                            let result =
                                result.map_err(|error| PromptError::Provider(Box::new(error)));
                            let _ = final_tx.send(result);
                            actor.active = None;
                            actor.finish_turn();
                            this.stage = AgentLoopStage::Handoff {
                                future: this.build_handoff_future(),
                            };
                        } else {
                            data.pending_calls = calls.into();
                            let call = data
                                .pending_calls
                                .pop_front()
                                .expect("the round produced tool calls");
                            let stage_call = call.clone();
                            this.stage = AgentLoopStage::Tool {
                                future: build_tool_future(*data, call),
                                call: stage_call,
                                final_tx,
                            };
                        }
                    }
                },
                AgentLoopStage::Tool {
                    mut future,
                    call,
                    final_tx,
                } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = AgentLoopStage::Tool {
                            future,
                            call,
                            final_tx,
                        };
                        return Poll::Pending;
                    }
                    Poll::Ready(ToolOutcome::Cancelled) => {
                        let _ = final_tx.send(Err(PromptError::Cancelled));
                        actor.active = None;
                        actor.finish_turn();
                        this.stage = AgentLoopStage::Handoff {
                            future: this.build_handoff_future(),
                        };
                    }
                    Poll::Ready(ToolOutcome::Completed { mut data, result }) => {
                        let output = match result {
                            Ok(value) => value.to_string(),
                            Err(error) => format!("tool error: {error}"),
                        };
                        let item = TranscriptItem::ToolResult {
                            call_id: call.id,
                            output,
                        };
                        let _ = actor.store.append(vec![item.clone()]);
                        data.messages.push(item);

                        if data.pending_calls.is_empty() {
                            this.stage = AgentLoopStage::Round {
                                future: build_round_future(*data),
                                final_tx,
                            };
                        } else {
                            let next_call = data
                                .pending_calls
                                .pop_front()
                                .expect("a pending tool call exists");
                            let stage_call = next_call.clone();
                            this.stage = AgentLoopStage::Tool {
                                future: build_tool_future(*data, next_call),
                                call: stage_call,
                                final_tx,
                            };
                        }
                    }
                },
                AgentLoopStage::Handoff { mut future } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = AgentLoopStage::Handoff { future };
                        return Poll::Pending;
                    }
                    Poll::Ready(_) => {
                        this.stage = AgentLoopStage::Idle;
                    }
                },
            }
        }
    }
}
#[cfg(test)]
mod tests;
