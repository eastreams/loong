//! Agent-side types for the `loong` product.

use std::{
    collections::VecDeque,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use context::ContextStore;
use contracts::provider::{Request, StreamItem};
use contracts::tool::ToolSpec;
use contracts::transcript::{Role, TranscriptItem};
use kernel::Facade;
use loac::prelude::*;
use loac::{ActorOwner, ActorRef};
use provider::{Provider, StreamError};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tool_host::{ToolError, ToolRegistry, ToolSnapshot};

mod builder;
mod channel;
mod channel_tool;
mod resource;
mod tool_set;

pub use builder::{AgentBuilder, BuildError};
pub use channel::{ChannelError, ChannelTarget};
pub use resource::{Resource, ResourceNeed, Resources, WorkspaceRoot};
pub use tool_set::{FileTools, ToolSet};

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
    Provider(Box<StreamError<Request>>),
}

/// Agent actor that composes context storage, an upstream provider, a tool
/// host, a workspace root, and an optional system prompt.
///
/// `P: Clone` keeps stream handlers able to capture the provider they were
/// started with, so switching the actor's provider never interrupts streams
/// that are already running.
pub struct Agent<C, P>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut> + Clone,
{
    store: C,
    provider: P,
    registry: ToolRegistry,
    subagents: Vec<Box<dyn crate::builder::SubagentSpawner<C, P>>>,
    system_prompt: Option<String>,
    /// Prompts that have been accepted by the mailbox but not started yet.
    ///
    /// All reads and writes happen in actor contexts (mailbox handlers and
    /// [`PromptLoop`] polls), so no lock is needed.
    prompt_queue: VecDeque<QueuedPrompt<P>>,
    /// Cancellation token for the prompt currently running, if any.
    ///
    /// The slot stays `Some` until the running prompt has observed its
    /// cancellation and handed the turn to the next queued prompt, which keeps
    /// new `Prompt` messages queued instead of overlapping the old loop.
    active: Option<CancellationToken>,
}

impl<C, P> Agent<C, P>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut> + Clone,
{
    /// Returns a builder that assembles one agent with explicit resources.
    ///
    /// `C` and `P` are inferred from [`AgentBuilder::with_store`] and
    /// [`AgentBuilder::with_provider`], so callers can write
    /// `Agent::builder(facade).with_store(store).with_provider(provider).build()?.spawn()`
    /// without naming the generic parameters.
    #[must_use]
    pub fn builder(facade: Facade) -> AgentBuilder<C, P> {
        AgentBuilder::<C, P>::new(facade)
    }

    /// Appends the user message, then snapshots everything the next prompt
    /// loop needs. Runs only in actor contexts, so preparation happens in
    /// mailbox order at the moment the prompt actually starts.
    fn prepare(&mut self, text: String, cancellation: CancellationToken) -> PreparedPrompt<P> {
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
    ///
    /// On success the next prompt future's oneshot fires with a prepared plan
    /// and `active` points at the new prompt's cancellation token. On a stale
    /// queue entry (its future is already gone) the entry is discarded without
    /// starting anything.
    fn start_next_prompt(&mut self) {
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

impl<C, P> Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    /// Starts this agent actor and returns its lifecycle owner.
    pub fn spawn(self) -> ActorOwner<Self> {
        loac::spawn::<Self>(self)
    }
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
#[message(stream = StreamItem, reply = Result<(), PromptError>)]
pub struct Prompt {
    /// The user message to append and send.
    pub text: String,
}

/// Cancels all prompts that are queued but not started yet.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelQueuedPrompts;

/// Cancels the active prompt, if any. The next queued prompt starts once the
/// cancelled prompt has observed its cancellation and handed the turn back.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelActivePrompt;

/// Cancels both queued prompts and the active prompt.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct CancelAllPrompts;

/// One-way self-message a finished prompt sends before its final value.
///
/// Sending this through the mailbox keeps next-prompt handoff ordered by the
/// actor, and admission failure is how a finishing prompt observes shutdown:
/// once admission is closed it clears the queue instead of starting new work.
#[derive(loac::Message)]
#[message(reply = ())]
struct PrepareNextPrompt<P>(PhantomData<fn() -> P>);

/// Owned inputs for one prompt loop.
struct PreparedPrompt<P> {
    messages: Vec<TranscriptItem>,
    provider: P,
    registry: Arc<ToolSnapshot>,
    system_prompt: Option<String>,
    cancellation: CancellationToken,
}

/// Queue entry for a prompt whose reply future is waiting for a start signal.
struct QueuedPrompt<P> {
    text: String,
    start_tx: oneshot::Sender<PreparedPrompt<P>>,
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

/// All mutable prompt-loop state, moved through the stage futures so the
/// [`PromptLoop`] struct itself stays [`Unpin`] regardless of `P` and `W`.
struct LoopData<P, W> {
    messages: Vec<TranscriptItem>,
    provider: P,
    registry: Arc<ToolSnapshot>,
    tools: Vec<ToolSpec>,
    cancellation: CancellationToken,
    pending_calls: VecDeque<PendingToolCall>,
    out: W,
}

impl<P, W> LoopData<P, W> {
    fn from_prepared(prepared: PreparedPrompt<P>, out: W) -> Self {
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

enum WaitStartOutcome<P, W> {
    Started(PreparedPrompt<P>, W),
    Cancelled,
}

enum RoundOutcome<P, W> {
    Completed {
        data: Box<LoopData<P, W>>,
        result: Result<(), StreamError<Request>>,
        items: Vec<StreamItem>,
    },
    Cancelled,
}

enum ToolOutcome<P, W> {
    Completed {
        data: Box<LoopData<P, W>>,
        result: Result<Value, ToolError>,
    },
    Cancelled,
}

type WaitStartFuture<P, W> = Pin<Box<dyn Future<Output = WaitStartOutcome<P, W>> + Send>>;
type RoundFuture<P, W> = Pin<Box<dyn Future<Output = RoundOutcome<P, W>> + Send>>;
type ToolFuture<P, W> = Pin<Box<dyn Future<Output = ToolOutcome<P, W>> + Send>>;
type HandoffFuture = Pin<Box<dyn Future<Output = bool> + Send>>;

enum PromptStage<P, W> {
    WaitStart {
        future: WaitStartFuture<P, W>,
    },
    Round {
        future: RoundFuture<P, W>,
    },
    Tool {
        future: ToolFuture<P, W>,
        call: PendingToolCall,
    },
    Handoff {
        future: HandoffFuture,
        result: Result<(), PromptError>,
    },
    Complete,
}

/// Actor-native prompt state machine.
///
/// It waits for the actor to prepare its plan, then alternates provider rounds
/// and tool invocations. Transcript commits happen directly through the actor
/// borrow in `poll`, never through the mailbox, so a graceful shutdown lets the
/// active prompt finish and commit completely. Cancellation is cooperative:
/// each stage future selects on the prompt's [`CancellationToken`].
struct PromptLoop<C, P, W>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    W: Writer<StreamItem> + Send + 'static,
{
    stage: PromptStage<P, W>,
    myself: ActorRef<Agent<C, P>>,
}

impl<C, P, W> PromptLoop<C, P, W>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    W: Writer<StreamItem> + Send + 'static,
{
    fn new(
        start_rx: oneshot::Receiver<PreparedPrompt<P>>,
        out: W,
        myself: ActorRef<Agent<C, P>>,
    ) -> Self {
        Self {
            stage: PromptStage::WaitStart {
                future: build_wait_start_future(start_rx, out),
            },
            myself,
        }
    }

    fn build_handoff_future(&self) -> HandoffFuture {
        let myself = self.myself.clone();
        Box::pin(async move {
            myself
                .send(PrepareNextPrompt::<P>(PhantomData))
                .await
                .is_ok()
        })
    }
}

fn build_wait_start_future<P, W>(
    start_rx: oneshot::Receiver<PreparedPrompt<P>>,
    out: W,
) -> WaitStartFuture<P, W>
where
    P: Send + 'static,
    W: Writer<StreamItem> + Send + 'static,
{
    Box::pin(async move {
        match start_rx.await {
            Ok(prepared) => WaitStartOutcome::Started(prepared, out),
            Err(_) => WaitStartOutcome::Cancelled,
        }
    })
}

fn build_round_future<P, W>(data: LoopData<P, W>) -> RoundFuture<P, W>
where
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
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

fn build_tool_future<P, W>(data: LoopData<P, W>, call: PendingToolCall) -> ToolFuture<P, W>
where
    P: Send + 'static,
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

#[actor(mailbox, interleaved = unbounded, children = unbounded)]
impl<C, P> Actor for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    type SpawnArgs = Self;

    async fn init(agent: Self::SpawnArgs, scope: &mut ActorScope<'_, Self>) -> Self {
        let mut agent = agent;
        for subagent in std::mem::take(&mut agent.subagents) {
            subagent.spawn(scope, &mut agent.registry);
        }
        agent
    }
}

impl<C, P> SyncHandler<SwitchProvider<P>> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, message: SwitchProvider<P>, _scope: &mut ActorScope<'_, Self>) {
        self.provider = message.0;
    }
}

impl<C, P> SyncHandler<PrepareNextPrompt<P>> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, _message: PrepareNextPrompt<P>, _scope: &mut ActorScope<'_, Self>) {
        self.start_next_prompt();
    }
}

impl<C, P> SyncHandler<CancelQueuedPrompts> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, _message: CancelQueuedPrompts, _scope: &mut ActorScope<'_, Self>) {
        self.prompt_queue.clear();
    }
}

impl<C, P> SyncHandler<CancelActivePrompt> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, _message: CancelActivePrompt, _scope: &mut ActorScope<'_, Self>) {
        if let Some(token) = &self.active {
            token.cancel();
        }
    }
}

impl<C, P> SyncHandler<CancelAllPrompts> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, _message: CancelAllPrompts, _scope: &mut ActorScope<'_, Self>) {
        self.prompt_queue.clear();
        if let Some(token) = &self.active {
            token.cancel();
        }
    }
}

impl<C, P> StreamHandler<Prompt> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle<W>(
        &mut self,
        message: Prompt,
        out: W,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, Prompt> + use<C, P, W>
    where
        W: loac::Writer<StreamItem> + Send + 'static,
    {
        let (start_tx, start_rx) = oneshot::channel();
        self.prompt_queue.push_back(QueuedPrompt {
            text: message.text,
            start_tx,
        });

        if self.active.is_none() {
            self.start_next_prompt();
        }

        let myself = scope.myself().clone();
        PromptLoop::new(start_rx, out, myself).interleaved()
    }
}

impl<C, P, W> ActorFuture<Agent<C, P>> for PromptLoop<C, P, W>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
    W: Writer<StreamItem> + Send + 'static,
{
    type Output = Result<(), PromptError>;

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut Agent<C, P>,
        _scope: &mut ActorScope<'_, Agent<C, P>>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            let mut stage = PromptStage::Complete;
            std::mem::swap(&mut this.stage, &mut stage);

            match stage {
                PromptStage::WaitStart { mut future } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = PromptStage::WaitStart { future };
                        return Poll::Pending;
                    }
                    Poll::Ready(WaitStartOutcome::Cancelled) => {
                        return Poll::Ready(Err(PromptError::Cancelled));
                    }
                    Poll::Ready(WaitStartOutcome::Started(prepared, out)) => {
                        let data = LoopData::from_prepared(prepared, out);
                        this.stage = PromptStage::Round {
                            future: build_round_future(data),
                        };
                    }
                },
                PromptStage::Round { mut future } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = PromptStage::Round { future };
                        return Poll::Pending;
                    }
                    Poll::Ready(RoundOutcome::Cancelled) => {
                        let result = Err(PromptError::Cancelled);
                        let handoff = this.build_handoff_future();
                        this.stage = PromptStage::Handoff {
                            future: handoff,
                            result,
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
                            let handoff = this.build_handoff_future();
                            this.stage = PromptStage::Handoff {
                                future: handoff,
                                result,
                            };
                        } else {
                            data.pending_calls = calls.into();
                            let call = data
                                .pending_calls
                                .pop_front()
                                .expect("the round produced tool calls");
                            let stage_call = call.clone();
                            this.stage = PromptStage::Tool {
                                future: build_tool_future(*data, call),
                                call: stage_call,
                            };
                        }
                    }
                },
                PromptStage::Tool { mut future, call } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = PromptStage::Tool { future, call };
                        return Poll::Pending;
                    }
                    Poll::Ready(ToolOutcome::Cancelled) => {
                        let result = Err(PromptError::Cancelled);
                        let handoff = this.build_handoff_future();
                        this.stage = PromptStage::Handoff {
                            future: handoff,
                            result,
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
                            this.stage = PromptStage::Round {
                                future: build_round_future(*data),
                            };
                        } else {
                            let next_call = data
                                .pending_calls
                                .pop_front()
                                .expect("a pending tool call exists");
                            let stage_call = next_call.clone();
                            this.stage = PromptStage::Tool {
                                future: build_tool_future(*data, next_call),
                                call: stage_call,
                            };
                        }
                    }
                },
                PromptStage::Handoff { mut future, result } => match future.as_mut().poll(cx) {
                    Poll::Pending => {
                        this.stage = PromptStage::Handoff { future, result };
                        return Poll::Pending;
                    }
                    Poll::Ready(true) => return Poll::Ready(result),
                    Poll::Ready(false) => {
                        actor.prompt_queue.clear();
                        return Poll::Ready(result);
                    }
                },
                PromptStage::Complete => unreachable!("Complete is only a poll placeholder"),
            }
        }
    }
}

#[cfg(test)]
mod tests;
