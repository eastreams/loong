# Channel API Abstraction Design

## Status

Draft - Pending Implementation

## Summary

Refactor the channel integration architecture to establish a platform-agnostic API abstraction layer. This enables:
- Consistent directory structure across all channel integrations (Telegram, Matrix, Feishu)
- Shared tool infrastructure via common traits
- Dependency inversion: tools depend on traits, not concrete implementations
- Extensibility for future integrations (Slack, Discord, etc.)

## Problem Statement

### Current Issues

| Issue | Description |
|-------|-------------|
| **Inconsistent Structure** | Feishu has two directories (`feishu/` and `channel/feishu/`) while Telegram/Matrix are single files |
| **Bidirectional Dependencies** | Both `tools/feishu.rs` and `channel/feishu/` depend on top-level `feishu/` module |
| **No API Reuse** | Each channel's tools are implemented independently without shared abstractions |
| **Tight Coupling** | Tools are bound to specific client implementations rather than interfaces |

### Root Cause

The current architecture lacks a clear separation between:
1. **Channel transport layer** (webhook/websocket/polling) - handles message delivery
2. **Platform API layer** (messaging/docs/calendar) - provides platform-specific capabilities
3. **Tool registration layer** - connects capabilities to the tool catalog

## Design Goals

1. **Structural Consistency**: All channels follow the same directory pattern
2. **API Abstraction**: Common capabilities expressed as traits
3. **Dependency Inversion**: Higher-level modules depend on abstractions
4. **Extensibility**: New channels implement existing traits without modifying tools layer
5. **Backward Compatibility**: Existing tool names, configs, and behaviors unchanged

## Architecture

### Dependency Flow

```text
┌─────────────────────────────────────────────────────────────────┐
│                           tools/                                 │
│              Generic tool handlers (messaging, docs, calendar)    │
│                         ▲                                       │
│                         │ depends on                            │
│                         │                                       │
│                         ▼                                       │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                    channel/traits/                          │ │
│  │  MessagingApi (mandatory) │ DocumentsApiAble │ CalendarApiAble │
│  └─────────────────────────────────────────────────────────────┘ │
│                         ▲                                       │
│                         │ implemented by                        │
│                         │                                       │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │               channel/{feishu,telegram,matrix}/impl/        │ │
│  │                    Concrete implementations                  │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                 channel/{feishu,telegram,matrix}/          │ │
│  │            ChannelAdapter impl (transport layer)            │ │
│  └─────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
```

### Directory Structure

```text
crates/app/src/
├── channel/
│   ├── mod.rs              ← re-exports channel types
│   ├── traits/
│   │   ├── mod.rs          ← re-exports all traits
│   │   ├── messaging.rs    ← MessagingApi + MessageContent + Pagination
│   │   ├── documents.rs    ← DocumentsApiAble + related types
│   │   └── calendar.rs     ← CalendarApiAble + TimeRange
│   ├── registry.rs         ← Channel registry
│   ├── runtime_state.rs    ← Runtime state tracking
│   ├── telegram/
│   │   ├── mod.rs          ← TelegramAdapter (ChannelAdapter impl)
│   │   └── impl/
│   │       ├── mod.rs
│   │       └── messaging.rs ← impl MessagingApi for TelegramClient
│   ├── matrix/
│   │   ├── mod.rs          ← MatrixAdapter (ChannelAdapter impl)
│   │   └── impl/
│   │       ├── mod.rs
│   │       └── messaging.rs ← impl MessagingApi for MatrixClient
│   └── feishu/
│       ├── mod.rs
│       ├── impl/
│       │   ├── mod.rs
│       │   ├── messaging.rs ← impl MessagingApi for FeishuClient
│       │   ├── documents.rs ← impl DocumentsApiAble
│       │   └── calendar.rs  ← impl CalendarApiAble
│       ├── client.rs        ← FeishuClient (HTTP client)
│       ├── adapter.rs       ← FeishuAdapter (ChannelAdapter impl)
│       ├── webhook.rs
│       ├── websocket.rs
│       └── payload/
│
└── tools/
    ├── mod.rs
    ├── channel.rs          ← Generic tool handlers + registration
    └── ...                 ← Other tools (shell, file, browser, etc.)
```

### Key Principles

1. **Mandatory vs Optional Traits**:
   - `MessagingApi` - All channels must implement (mandatory)
   - `DocumentsApiAble`, `CalendarApiAble` - Implement only if supported (optional)

2. **Single Responsibility**:
   - Each channel directory contains only channel-specific code
   - Shared abstractions live in `traits/`
   - Generic tools live in `tools/`

3. **Feature Gating**:
   - Optional traits are gated by corresponding feature flags
   - Tool registration checks trait availability at runtime

## Traits Design

### MessagingApi (Mandatory)

All channel adapters MUST implement this trait.

```rust
pub trait MessagingApi: Send + Sync {
    type Receipt: Send + Sync;
    type Message: Send + Sync;
    type MessagePage: Send + Sync;
    type SendOptions: Default + Send + Sync;

    async fn send_message(
        &self,
        target: &str,
        receive_id_type: Option<&str>,
        content: &MessageContent,
        idempotency_key: Option<&str>,
    ) -> ApiResult<Self::Receipt>;

    async fn reply(
        &self,
        parent_id: &str,
        content: &MessageContent,
    ) -> ApiResult<Self::Receipt>;

    async fn get_message(&self, message_id: &str) -> ApiResult<Self::Message>;

    async fn list_messages(
        &self,
        chat_id: &str,
        pagination: &Pagination,
    ) -> ApiResult<Self::MessagePage>;

    async fn upload_media(
        &self,
        file_path: Option<&str>,
        file_key: Option<&str>,
        media_type: MediaType,
    ) -> ApiResult<MediaUploadResult>;
}

#[derive(Clone, Debug)]
pub struct MessageContent {
    pub text: Option<String>,
    pub html: Option<String>,
    pub image_key: Option<String>,
    pub file_key: Option<String>,
    pub file_type: Option<String>,
    pub card: Option<serde_json::Value>,
}

#[derive(Clone, Default, Debug)]
pub struct Pagination {
    pub page_size: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub enum MediaType {
    Image,
    File,
    Audio,
    Video,
}
```

### DocumentsApiAble (Optional)

Implement this trait if the platform supports document operations.

```rust
pub trait DocumentsApiAble: Send + Sync {
    type Document: Send + Sync;
    type DocumentContent: Send + Sync;

    async fn create_document(
        &self,
        title: &str,
        content: Option<&str>,
    ) -> ApiResult<Self::Document>;

    async fn read_document(&self, doc_id: &str) -> ApiResult<Self::DocumentContent>;

    async fn append_to_document(
        &self,
        doc_id: &str,
        content: &str,
    ) -> ApiResult<()>;
}
```

### CalendarApiAble (Optional)

Implement this trait if the platform supports calendar operations.

```rust
pub trait CalendarApiAble: Send + Sync {
    type Calendar: Send + Sync;
    type CalendarList: Send + Sync;
    type FreeBusyResult: Send + Sync;

    async fn list_calendars(&self) -> ApiResult<Self::CalendarList>;

    async fn get_primary_calendar(&self) -> ApiResult<Self::Calendar>;

    async fn query_freebusy(
        &self,
        time_range: &TimeRange,
        participants: &[String],
    ) -> ApiResult<Self::FreeBusyResult>;
}

#[derive(Clone, Debug)]
pub struct TimeRange {
    pub start_timestamp: i64,
    pub end_timestamp: i64,
}
```

### Error Handling

```rust
pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("rate limited, retry after {0}s")]
    RateLimited(u64),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("platform error: code={code}, message={message}")]
    Platform { code: i32, message: String },
}
```

## Generic Tools Layer

### Tool Handlers

```rust
// tools/channel.rs

pub mod channel {
    use super::*;
    use crate::channel::traits::*;

    pub async fn send<C: MessagingApi>(
        api: &C,
        target: &str,
        content: &MessageContent,
    ) -> ApiResult<C::Receipt> {
        api.send_message(target, None, content, None).await
    }

    pub async fn reply<C: MessagingApi>(
        api: &C,
        parent_id: &str,
        content: &MessageContent,
    ) -> ApiResult<C::Receipt> {
        api.reply(parent_id, content).await
    }

    pub async fn list<C: MessagingApi>(
        api: &C,
        chat_id: &str,
        page_size: Option<u32>,
    ) -> ApiResult<C::MessagePage> {
        api.list_messages(chat_id, &Pagination { page_size, cursor: None }).await
    }

    pub async fn create_doc<C: DocumentsApiAble>(
        api: &C,
        title: &str,
        content: Option<&str>,
    ) -> ApiResult<C::Document> {
        api.create_document(title, content).await
    }

    pub async fn read_doc<C: DocumentsApiAble>(
        api: &C,
        doc_id: &str,
    ) -> ApiResult<C::DocumentContent> {
        api.read_document(doc_id).await
    }

    pub async fn list_calendars<C: CalendarApiAble>(
        api: &C,
    ) -> ApiResult<C::CalendarList> {
        api.list_calendars().await
    }

    pub async fn freebusy<C: CalendarApiAble>(
        api: &C,
        range: &TimeRange,
        participants: &[String],
    ) -> ApiResult<C::FreeBusyResult> {
        api.query_freebusy(range, participants).await
    }
}
```

### Registration Pattern

```rust
pub fn register_messaging_tools<C: MessagingApi + 'static>(
    catalog: &mut ToolCatalog,
    platform: &str,
    api: Arc<C>,
) {
    let send_name = format!("{}.send", platform);
    let reply_name = format!("{}.reply", platform);
    let list_name = format!("{}.list", platform);

    catalog.register(send_name, move |target, content| {
        let api = api.clone();
        async move { channel::send(api.as_ref(), target, content).await }
    });

    // ... register reply, list
}

pub fn register_documents_tools<C: DocumentsApiAble + 'static>(
    catalog: &mut ToolCatalog,
    platform: &str,
    api: Arc<C>,
) {
    // ... only registered if DocumentsApiAble is implemented
}

pub fn register_calendar_tools<C: CalendarApiAble + 'static>(
    catalog: &mut ToolCatalog,
    platform: &str,
    api: Arc<C>,
) {
    // ... only registered if CalendarApiAble is implemented
}
```

## Migration Plan

### Phase 1: Create Traits Module

**Duration**: Low risk, foundational

1. Create `channel/traits/` directory
2. Define `MessagingApi` with associated types and required methods
3. Define `DocumentsApiAble` and `CalendarApiAble` with platform-specific types
4. Define `ApiResult<T>` and `ApiError` types
5. Create `channel/traits/mod.rs` with re-exports
6. **Verification**: Compile with `--all-features`, existing code unchanged

### Phase 2: Restructure Telegram

**Duration**: Medium risk, isolated change

1. Create `channel/telegram/` directory
2. Move `telegram.rs` → `telegram/mod.rs`
3. Create `telegram/impl/` directory
4. Implement `MessagingApi` for Telegram client in `impl/messaging.rs`
5. Update `channel/mod.rs` imports
6. **Verification**: `cargo test --features channel-telegram`

### Phase 3: Restructure Matrix

**Duration**: Medium risk, isolated change

1. Create `channel/matrix/` directory
2. Move `matrix.rs` → `matrix/mod.rs`
3. Create `matrix/impl/` directory
4. Implement `MessagingApi` for Matrix client in `impl/messaging.rs`
5. Update `channel/mod.rs` imports
6. **Verification**: `cargo test --features channel-matrix`

### Phase 4: Restructure Feishu

**Duration**: High risk, largest change

1. Create `channel/feishu/` directory (may already exist with `channel/`)
2. Move all files from top-level `feishu/` to `channel/feishu/`
3. Create `feishu/impl/` directory with:
   - `messaging.rs` - impl MessagingApi
   - `documents.rs` - impl DocumentsApiAble
   - `calendar.rs` - impl CalendarApiAble
4. Update all import paths within `feishu/`
5. Update `channel/mod.rs` imports
6. Remove top-level `feishu/` module from `lib.rs`
7. Remove `#[cfg(feature = "feishu-integration")] pub mod feishu`
8. **Verification**: `cargo test --features feishu-integration,channel-feishu`

### Phase 5: Create Generic Tools

**Duration**: Medium risk, new functionality

1. Create `tools/channel.rs` with generic tool handlers
2. Implement registration functions for each trait group
3. Delete `tools/feishu.rs`
4. Update tool catalog initialization
5. **Verification**: All tool names work identically, output unchanged

### Phase 6: Integration Testing

**Duration**: Low risk, verification

1. Run full test suite: `cargo test --workspace --all-features`
2. Run format check: `cargo fmt --all -- --check`
3. Run clippy: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
4. Verify feature flags work correctly in all combinations

## Backward Compatibility

| Aspect | Strategy |
|--------|----------|
| **Tool Names** | Names like `feishu.messages.send` unchanged via registration |
| **API Responses** | Associated types ensure exact response shapes match |
| **Configuration** | No changes to config files or parsing |
| **Feature Flags** | New structure matches existing feature gates |

## Open Questions

### 1. Pagination Design

**Option A**: Generic `Pagination` struct with optional fields (current design)
```rust
pub struct Pagination {
    pub page_size: Option<u32>,
    pub cursor: Option<String>,
}
```
**Pros**: Simple, platform-agnostic
**Cons**: Some platforms use page_token, others use cursor

**Option B**: Associated types per platform
```rust
trait MessagingApi {
    type Pagination: PaginationStrategy;
}
```
**Pros**: Platform-specific optimization
**Cons**: More complex, harder to use generically

**Recommendation**: Option A for MVP, migrate to Option B if needed

### 2. Platform-Specific Features

Examples:
- Feishu: card updates, custom emoji reactions
- Telegram: inline keyboards, callback queries
- Matrix: room encryption, mentions

**Options**:
1. Separate traits: `CardSupport`, `InlineKeyboardSupport`
2. Extension trait methods with default implementations
3. Platform-specific tool registries

**Recommendation**: Extension traits with default no-op implementations

### 3. Authentication Strategy

Each platform has different auth mechanisms:
- Feishu: OAuth 2.0 with user tokens + tenant tokens
- Telegram: Bot tokens
- Matrix: Access tokens + homeserver URL

**Current approach**: Keep auth in channel-specific config
**Question**: Should auth be abstracted into a trait?

**Recommendation**: Keep auth platform-specific for now; revisit if patterns emerge

### 4. Transaction/Batch Semantics

How to handle:
- Optimistic updates
- Rollback on failure
- Batch operations

**Recommendation**: Out of scope for initial refactoring

## Risk Assessment

| Phase | Risk Level | Mitigation |
|-------|------------|------------|
| Phase 1: Traits Module | Low | Pure addition, no changes to existing code |
| Phase 2: Telegram | Medium | Work in feature flag, extensive testing |
| Phase 3: Matrix | Medium | Same as Phase 2 |
| Phase 4: Feishu | High | Largest refactoring, most imports to update |
| Phase 5: Generic Tools | Medium | New functionality, can feature-flag |

## Testing Strategy

### Unit Tests
- Each trait implementation has corresponding unit tests
- Mock implementations for trait boundaries
- Property-based tests for serialization

### Integration Tests
- End-to-end tests with mocked platform APIs
- Verify tool responses match current behavior
- Feature flag combinations tested

### Migration Tests
- Before/after response comparison
- Snapshot tests for API responses
- Configuration parsing unchanged

## References

| Component | Location |
|-----------|----------|
| ChannelAdapter trait | `crates/app/src/channel/mod.rs` |
| Current FeishuClient | `crates/app/src/feishu/client.rs` (to be moved) |
| Current Feishu tools | `crates/app/src/tools/feishu.rs` (to be deleted) |
| Current Telegram adapter | `crates/app/src/channel/telegram.rs` (to be restructured) |
| Current Matrix adapter | `crates/app/src/channel/matrix.rs` (to be restructured) |

## Changelog

| Date | Version | Changes |
|------|---------|---------|
| 2026-03-21 | 0.0.1 | Initial draft |
