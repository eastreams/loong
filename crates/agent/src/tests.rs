use super::*;
use async_trait::async_trait;
use context::{ContextSnapshot, ContextStore, memory::MemoryStore};
use contracts::transcript::{Role, TranscriptItem};
use loac::{ExitReason, Shutdown, Writer};
use provider::{Provider, StreamError};
use std::sync::{Arc, Mutex};

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
    let owner =
        loac::spawn::<Agent<MemoryStore, DummyProvider>>((MemoryStore::new(), DummyProvider));
    let actor_ref = owner.actor_ref();

    actor_ref.call(SwitchProvider(DummyProvider)).await.unwrap();

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
}

#[tokio::test]
async fn switch_provider_accepts_arc_of_non_clone_provider() {
    let owner = loac::spawn::<Agent<MemoryStore, Arc<NonCloneProvider>>>((
        MemoryStore::new(),
        Arc::new(NonCloneProvider),
    ));
    let actor_ref = owner.actor_ref();

    actor_ref
        .call(SwitchProvider(Arc::new(NonCloneProvider)))
        .await
        .unwrap();

    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
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
    let store = SharedStore(Arc::new(Mutex::new(MemoryStore::new())));
    let owner = loac::spawn::<Agent<SharedStore, EchoProvider>>((store.clone(), EchoProvider));
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
}
