use super::*;
use async_trait::async_trait;
use context::{ContextSnapshot, ContextStore, memory::MemoryStore};
use contracts::transcript::{Role, TranscriptItem};
use kernel::{Facade, Kernel, policy::engine::PolicyEngine};
use loac::{ExitReason, Shutdown, Writer};
use provider::{Provider, StreamError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

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

fn plan_app() -> (loac::ActorOwner<Kernel>, App) {
    let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let profile = PlanProfile;
    let facade = Facade::for_owner(&kernel_owner, profile.capabilities());
    let app = App::new(facade);
    (kernel_owner, app)
}

fn file_io_app() -> (loac::ActorOwner<Kernel>, App) {
    let kernel_owner = loac::spawn::<Kernel>(PolicyEngine::allow_capabilities());
    let profile = FileIoProfile;
    let facade = Facade::for_owner(&kernel_owner, profile.capabilities());
    let mut app = App::new(facade);
    profile.register_tools(&mut app);
    (kernel_owner, app)
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
    let (kernel_owner, app) = plan_app();
    let workspace = temp_workspace();
    let owner = loac::spawn::<PlanAgent<MemoryStore, DummyProvider>>((
        MemoryStore::new(),
        DummyProvider,
        app,
        workspace,
        PlanProfile,
    ));
    let actor_ref = owner.actor_ref();

    actor_ref.call(SwitchProvider(DummyProvider)).await.unwrap();

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    let _ = kernel_owner.shutdown(Shutdown::Drain).await;
}

#[tokio::test]
async fn switch_provider_accepts_arc_of_non_clone_provider() {
    let (kernel_owner, app) = plan_app();
    let workspace = temp_workspace();
    let owner = loac::spawn::<PlanAgent<MemoryStore, Arc<NonCloneProvider>>>((
        MemoryStore::new(),
        Arc::new(NonCloneProvider),
        app,
        workspace,
        PlanProfile,
    ));
    let actor_ref = owner.actor_ref();

    actor_ref
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
    let (kernel_owner, app) = plan_app();
    let workspace = temp_workspace();
    let store = SharedStore(Arc::new(Mutex::new(MemoryStore::new())));
    let owner = loac::spawn::<PlanAgent<SharedStore, EchoProvider>>((
        store.clone(),
        EchoProvider,
        app,
        workspace,
        PlanProfile,
    ));
    let actor_ref = owner.actor_ref();

    let mut reply = actor_ref
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
    let (kernel_owner, app) = file_io_app();
    let workspace = temp_workspace();
    std::fs::write(workspace.join("hello.txt"), "hello").unwrap();

    let store = SharedStore(Arc::new(Mutex::new(MemoryStore::new())));
    let owner = loac::spawn::<FileIoAgent<SharedStore, ToolCallProvider>>((
        store.clone(),
        ToolCallProvider {
            calls: Arc::new(AtomicUsize::new(0)),
        },
        app,
        workspace,
        FileIoProfile,
    ));
    let actor_ref = owner.actor_ref();

    let mut reply = actor_ref
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
