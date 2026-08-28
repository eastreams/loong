use super::*;
use async_trait::async_trait;
use context::{ContextSnapshot, ContextStore, memory::MemoryStore};
use contracts::capability::{Capabilities, Capability};
use contracts::transcript::{Role, TranscriptItem};
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
use loac::{ExitReason, Shutdown, Writer};
use provider::{Provider, StreamError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, watch};

const PLAN_SYSTEM_PROMPT: &str = "You are a planning agent. Produce concise, ordered plans.";
const FILE_IO_SYSTEM_PROMPT: &str =
    "You are a file I/O agent. Use read_file and write_file for workspace files.";

fn temp_workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "loong-agent-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn plan_facade() -> (loac::ActorOwner<Kernel>, Facade) {
    let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let facade = Facade::new(kernel_owner.actor_ref(), Capabilities::empty());
    (kernel_owner, facade)
}

fn file_io_facade() -> (loac::ActorOwner<Kernel>, Facade) {
    let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let capabilities = Capabilities::empty()
        .with(Capability::FsRead)
        .with(Capability::FsWrite);
    let facade = Facade::new(kernel_owner.actor_ref(), capabilities);
    (kernel_owner, facade)
}

#[derive(Clone)]
struct DummyProvider;

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for DummyProvider {
    async fn stream(
        &self,
        _req: Request,
        _out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        Ok(())
    }
}

/// A provider that is intentionally not `Clone`; it can still be used
/// with `Agent` by wrapping it in an `Arc`, which is `Clone` and now
/// implements `Provider` through the blanket impl.
struct NonCloneProvider;

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for NonCloneProvider {
    async fn stream(
        &self,
        _req: Request,
        _out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        Ok(())
    }
}

#[tokio::test]
async fn switch_provider_message_is_accepted() {
    let (kernel_owner, facade) = plan_facade();
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(DummyProvider)
        .build()
        .unwrap()
        .spawn();

    owner.call(SwitchProvider(DummyProvider)).await.unwrap();

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn switch_provider_accepts_arc_of_non_clone_provider() {
    let (kernel_owner, facade) = plan_facade();
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(Arc::new(NonCloneProvider))
        .build()
        .unwrap()
        .spawn();

    owner
        .call(SwitchProvider(Arc::new(NonCloneProvider)))
        .await
        .unwrap();

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[derive(Clone)]
struct SharedStore(Arc<Mutex<MemoryStore>>);

impl ContextStore for SharedStore {
    fn append(&mut self, items: Vec<TranscriptItem>) -> u64 {
        self.0.lock().unwrap().append(items)
    }

    fn replace(&mut self, items: Vec<TranscriptItem>) -> u64 {
        self.0.lock().unwrap().replace(items)
    }

    fn version(&self) -> u64 {
        self.0.lock().unwrap().version()
    }

    fn snapshot(&self) -> ContextSnapshot {
        self.0.lock().unwrap().snapshot()
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

#[derive(Clone)]
struct ToolCallProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for ToolCallProvider {
    async fn stream(
        &self,
        _req: Request,
        out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            out.write(StreamItem::ToolCall {
                id: "call_1".to_string(),
                name: "read_file".to_string(),
                arguments: "{\"path\":\"hello.txt\"}".to_string(),
            })
            .await
            .unwrap();
        } else {
            out.write(StreamItem::Text {
                delta: "hello".to_string(),
            })
            .await
            .unwrap();
        }
        Ok(())
    }
}

#[derive(Clone)]
struct EchoProvider;

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for EchoProvider {
    async fn stream(
        &self,
        _req: Request,
        out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        out.write(StreamItem::Text {
            delta: "hello".to_string(),
        })
        .await
        .unwrap();
        Ok(())
    }
}

#[tokio::test]
async fn prompt_streams_and_appends_context() {
    let (kernel_owner, facade) = plan_facade();
    let store = SharedStore(Arc::new(Mutex::new(MemoryStore::new())));
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(store.clone())
        .with_provider(EchoProvider)
        .build()
        .unwrap()
        .spawn();

    let mut reply = owner
        .call(Prompt {
            text: "hi".to_string(),
        })
        .await
        .unwrap();

    let mut text = String::new();
    while let Some(item) = reply.recv().await {
        match item {
            StreamItem::Text { delta } => text.push_str(&delta),
            StreamItem::ToolCall { .. } => panic!("unexpected tool call"),
        }
    }
    reply.finish().await.unwrap().unwrap();

    assert_eq!(text, "hello");

    let snapshot = store.snapshot();
    assert_eq!(snapshot.items.len(), 2);
    assert_eq!(
        snapshot.items[0],
        TranscriptItem::Message {
            role: Role::User,
            text: "hi".to_string(),
        }
    );
    assert_eq!(
        snapshot.items[1],
        TranscriptItem::Message {
            role: Role::Assistant,
            text: "hello".to_string(),
        }
    );

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn prompt_executes_tool_calls_and_continues() {
    let (kernel_owner, facade) = file_io_facade();
    let workspace = temp_workspace();
    std::fs::write(workspace.join("hello.txt"), "hello").unwrap();

    let store = SharedStore(Arc::new(Mutex::new(MemoryStore::new())));
    let owner = Agent::builder(facade)
        .with(FileTools)
        .with_workspace_root(&workspace)
        .with_system_prompt(FILE_IO_SYSTEM_PROMPT)
        .with_store(store.clone())
        .with_provider(ToolCallProvider {
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .build()
        .unwrap()
        .spawn();

    let mut reply = owner
        .call(Prompt {
            text: "read hello.txt".to_string(),
        })
        .await
        .unwrap();

    let mut text = String::new();
    let mut tool_names = Vec::new();
    while let Some(item) = reply.recv().await {
        match item {
            StreamItem::Text { delta } => text.push_str(&delta),
            StreamItem::ToolCall { name, .. } => tool_names.push(name),
        }
    }
    reply.finish().await.unwrap().unwrap();

    assert_eq!(text, "hello");
    assert_eq!(tool_names, vec!["read_file".to_string()]);

    let snapshot = store.snapshot();
    assert_eq!(snapshot.items.len(), 4);
    assert_eq!(
        snapshot.items[0],
        TranscriptItem::Message {
            role: Role::User,
            text: "read hello.txt".to_string(),
        }
    );
    assert_eq!(
        snapshot.items[1],
        TranscriptItem::ToolCall {
            call_id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: "{\"path\":\"hello.txt\"}".to_string(),
        }
    );
    assert_eq!(
        snapshot.items[2],
        TranscriptItem::ToolResult {
            call_id: "call_1".to_string(),
            output: "{\"content\":\"hello\"}".to_string(),
        }
    );
    assert_eq!(
        snapshot.items[3],
        TranscriptItem::Message {
            role: Role::Assistant,
            text: "hello".to_string(),
        }
    );

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn channel_target_ask_collects_streamed_text() {
    let (kernel_owner, facade) = plan_facade();
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(EchoProvider)
        .build()
        .unwrap()
        .spawn();

    let answer = owner.ask("hi".to_string()).await.unwrap();
    assert_eq!(answer, "hello");

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn builder_rejects_file_tools_without_workspace_root() {
    let (kernel_owner, facade) = file_io_facade();

    let result = Agent::builder(facade)
        .with(FileTools)
        .with_store(MemoryStore::new())
        .with_provider(DummyProvider)
        .build();

    assert!(matches!(result, Err(BuildError::MissingResource { .. })));

    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[derive(Clone)]
struct GatedProvider {
    started: Arc<watch::Sender<bool>>,
    gate: Arc<Notify>,
}

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for GatedProvider {
    async fn stream(
        &self,
        _req: Request,
        out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        let _ = self.started.send(true);
        out.write(StreamItem::Text {
            delta: "partial".to_string(),
        })
        .await
        .unwrap();
        self.gate.notified().await;
        out.write(StreamItem::Text {
            delta: "done".to_string(),
        })
        .await
        .unwrap();
        Ok(())
    }
}

#[derive(Clone)]
struct RecordingProvider {
    starts: Arc<Mutex<Vec<String>>>,
    gate: Arc<Notify>,
}

#[async_trait]
impl Provider<Request, StreamItem, ProviderOut> for RecordingProvider {
    async fn stream(
        &self,
        req: Request,
        out: &mut ProviderOut,
    ) -> Result<(), StreamError<Request>> {
        let user = match req.messages.last() {
            Some(TranscriptItem::Message { text, .. }) => text.clone(),
            _ => String::new(),
        };
        self.starts.lock().unwrap().push(user);
        self.gate.notified().await;
        out.write(StreamItem::Text {
            delta: "done".to_string(),
        })
        .await
        .unwrap();
        Ok(())
    }
}

#[tokio::test]
async fn cancel_active_prompt_interrupts_it_with_cancelled() {
    let (kernel_owner, facade) = plan_facade();
    let (started_tx, started_rx) = watch::channel(false);
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(GatedProvider {
            started: Arc::new(started_tx),
            gate: Arc::new(Notify::new()),
        })
        .build()
        .unwrap()
        .spawn();

    let mut reply = owner
        .call(Prompt {
            text: "hi".to_string(),
        })
        .await
        .unwrap();

    let mut started_rx = started_rx;
    started_rx.wait_for(|started| *started).await.unwrap();

    let item = reply.recv().await.unwrap();
    assert!(matches!(item, StreamItem::Text { .. }));

    owner.call(CancelActivePrompt).await.unwrap();

    while reply.recv().await.is_some() {}
    match reply.finish().await.unwrap() {
        Err(PromptError::Cancelled) => {}
        other => panic!("expected Cancelled, got {other:?}"),
    }

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn cancel_queued_prompt_returns_cancelled() {
    let (kernel_owner, facade) = plan_facade();
    let (started_tx, started_rx) = watch::channel(false);
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(GatedProvider {
            started: Arc::new(started_tx),
            gate: Arc::new(Notify::new()),
        })
        .build()
        .unwrap()
        .spawn();

    let mut first = owner
        .call(Prompt {
            text: "first".to_string(),
        })
        .await
        .unwrap();

    let mut started_rx = started_rx;
    started_rx.wait_for(|started| *started).await.unwrap();
    let _ = first.recv().await.unwrap();

    let mut second = owner
        .call(Prompt {
            text: "second".to_string(),
        })
        .await
        .unwrap();

    owner.call(CancelQueuedPrompts).await.unwrap();

    while second.recv().await.is_some() {}
    match second.finish().await.unwrap() {
        Err(PromptError::Cancelled) => {}
        other => panic!("expected queued prompt to be Cancelled, got {other:?}"),
    }

    owner.call(CancelActivePrompt).await.unwrap();
    while first.recv().await.is_some() {}
    match first.finish().await.unwrap() {
        Err(PromptError::Cancelled) => {}
        other => panic!("expected active prompt to be Cancelled, got {other:?}"),
    }

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn queued_prompts_start_in_fifo_order() {
    let (kernel_owner, facade) = plan_facade();
    let starts = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(Notify::new());
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(RecordingProvider {
            starts: Arc::clone(&starts),
            gate: Arc::clone(&gate),
        })
        .build()
        .unwrap()
        .spawn();

    let mut first = owner
        .call(Prompt {
            text: "first".to_string(),
        })
        .await
        .unwrap();
    let mut second = owner
        .call(Prompt {
            text: "second".to_string(),
        })
        .await
        .unwrap();
    let mut third = owner
        .call(Prompt {
            text: "third".to_string(),
        })
        .await
        .unwrap();

    wait_for_starts(&starts, 1).await;
    gate.notify_one();
    wait_for_starts(&starts, 2).await;

    let mut text = String::new();
    while let Some(item) = first.recv().await {
        if let StreamItem::Text { delta } = item {
            text.push_str(&delta);
        }
    }
    first.finish().await.unwrap().unwrap();
    assert_eq!(text, "done");

    gate.notify_one();
    wait_for_starts(&starts, 3).await;

    let mut text = String::new();
    while let Some(item) = second.recv().await {
        if let StreamItem::Text { delta } = item {
            text.push_str(&delta);
        }
    }
    second.finish().await.unwrap().unwrap();
    assert_eq!(text, "done");

    gate.notify_one();
    let mut text = String::new();
    while let Some(item) = third.recv().await {
        if let StreamItem::Text { delta } = item {
            text.push_str(&delta);
        }
    }
    third.finish().await.unwrap().unwrap();
    assert_eq!(text, "done");

    assert_eq!(
        *starts.lock().unwrap(),
        vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string()
        ]
    );

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn drain_finishes_active_and_cancels_queued() {
    let (kernel_owner, facade) = plan_facade();
    let (started_tx, started_rx) = watch::channel(false);
    let gate = Arc::new(Notify::new());
    let owner = Agent::builder(facade)
        .with_system_prompt(PLAN_SYSTEM_PROMPT)
        .with_store(MemoryStore::new())
        .with_provider(GatedProvider {
            started: Arc::new(started_tx),
            gate: Arc::clone(&gate),
        })
        .build()
        .unwrap()
        .spawn();

    let mut first = owner
        .call(Prompt {
            text: "first".to_string(),
        })
        .await
        .unwrap();

    let mut started_rx = started_rx;
    started_rx.wait_for(|started| *started).await.unwrap();
    let mut text = String::new();
    let item = first.recv().await.unwrap();
    if let StreamItem::Text { delta } = item {
        text.push_str(&delta);
    }
    assert_eq!(text, "partial");

    let mut second = owner
        .call(Prompt {
            text: "second".to_string(),
        })
        .await
        .unwrap();

    // Close admission while `first` is still active and `second` is queued.
    owner.request_shutdown(Shutdown::Drain);

    // Let the active prompt finish and commit directly; its handoff then
    // observes the closed admission and cancels the queue instead of starting
    // the next prompt.
    gate.notify_one();

    while let Some(item) = first.recv().await {
        if let StreamItem::Text { delta } = item {
            text.push_str(&delta);
        }
    }
    first.finish().await.unwrap().unwrap();
    assert_eq!(text, "partialdone");

    while second.recv().await.is_some() {}
    match second.finish().await.unwrap() {
        Err(PromptError::Cancelled) => {}
        other => panic!("expected queued prompt to be Cancelled during Drain, got {other:?}"),
    }

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

async fn wait_for_starts(starts: &Arc<Mutex<Vec<String>>>, len: usize) {
    for _ in 0..1000 {
        if starts.lock().unwrap().len() >= len {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for {len} started prompt(s)");
}
