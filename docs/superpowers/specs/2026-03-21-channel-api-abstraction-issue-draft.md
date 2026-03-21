# [Refactor] Establish Channel API Abstraction Layer, Unify Integration Architecture

## Status

Ready for implementation

## Problem Statement

The current channel integration architecture has the following issues:

### 1. Inconsistent Structure

| Channel | Current Structure | Issue |
|---------|-------------------|-------|
| Feishu | `feishu/` (top-level) + `channel/feishu/` (adapter) | Two directories, unclear responsibilities |
| Telegram | `channel/telegram.rs` | Single file |
| Matrix | `channel/matrix.rs` | Single file |

### 2. Chaotic Dependencies

```
tools/feishu.rs ──────► feishu/ (top-level module)
                           ├── FeishuClient
                           ├── resources/ (docs, calendar, messages...)
                           └── outbound.rs

channel/feishu/ ──────► feishu/
                           └── adapter.rs
```

Both consumers depend on `feishu/`, but it belongs to neither.

### 3. No API Reuse

Each channel's tools are implemented independently with no shared abstractions. Adding a new channel (e.g., Slack, Discord) requires duplicating effort.

## Proposed Solution

Establish a **Platform-Agnostic API Abstraction Layer** with dependency inversion:

```
                    ┌─────────────────────────┐
                    │         tools/          │
                    │   Generic tool handlers │
                    └───────────┬─────────────┘
                                │ depends on
                                ▼
┌─────────────────────────────────────────────────────────────┐
│                  channel/traits/                             │
│  MessagingApi │ DocumentsApiAble │ CalendarApiAble           │
└─────────────────────────────────────────────────────────────┘
                                ▲
                                │ implemented by
                                ▼
┌─────────────────────────────────────────────────────────────┐
│             channel/{feishu,telegram,matrix}/impl/           │
│                  Concrete implementations                    │
└─────────────────────────────────────────────────────────────┘
```

### Directory Structure

```
channel/
├── traits/
│   ├── mod.rs
│   ├── messaging.rs    ← MessagingApi (mandatory)
│   ├── documents.rs    ← DocumentsApiAble (optional)
│   └── calendar.rs     ← CalendarApiAble (optional)
├── telegram/
│   ├── mod.rs
│   └── impl/
│       └── messaging.rs ← impl MessagingApi
├── matrix/
│   ├── mod.rs
│   └── impl/
│       └── messaging.rs ← impl MessagingApi
└── feishu/
    ├── mod.rs
    ├── impl/
    │   ├── messaging.rs ← impl MessagingApi
    │   ├── documents.rs ← impl DocumentsApiAble
    │   └── calendar.rs   ← impl CalendarApiAble
    ├── client.rs
    ├── adapter.rs
    └── ...
```

### Traits Design

```rust
// MessagingApi - All channels MUST implement
pub trait MessagingApi: Send + Sync {
    type Receipt;
    type Message;
    type MessagePage;
    
    async fn send_message(&self, target: &str, content: &MessageContent, ...) -> ApiResult<Self::Receipt>;
    async fn reply(&self, parent_id: &str, content: &MessageContent) -> ApiResult<Self::Receipt>;
    async fn get_message(&self, id: &str) -> ApiResult<Self::Message>;
    async fn list_messages(&self, channel: &str, pagination: &Pagination) -> ApiResult<Self::MessagePage>;
}

// DocumentsApiAble - Optional implementation
pub trait DocumentsApiAble: Send + Sync {
    async fn create_document(&self, title: &str, content: Option<&str>) -> ApiResult<Self::Document>;
    async fn read_document(&self, id: &str) -> ApiResult<Self::DocumentContent>;
    async fn append_to_document(&self, id: &str, content: &str) -> ApiResult<()>;
}

// CalendarApiAble - Optional implementation
pub trait CalendarApiAble: Send + Sync {
    async fn list_calendars(&self) -> ApiResult<Self::CalendarList>;
    async fn query_freebusy(&self, range: &TimeRange, participants: &[String]) -> ApiResult<Self::FreeBusyResult>;
}
```

## Implementation Phases

### Phase 1: Create Traits Module
- Create `channel/traits/`
- Define `MessagingApi`, `DocumentsApiAble`, `CalendarApiAble`
- Define `ApiResult<T>` and `ApiError`

### Phase 2: Restructure Telegram
- Convert to directory structure
- Implement `MessagingApi`

### Phase 3: Restructure Matrix
- Convert to directory structure
- Implement `MessagingApi`

### Phase 4: Restructure Feishu
- Move all files to `channel/feishu/`
- Delete top-level `feishu/` module
- Implement all three traits

### Phase 5: Generalize Tools Layer
- Delete `tools/feishu.rs`
- Create `tools/channel.rs` with generic handlers
- Tool registration supports optional traits

### Phase 6: Integration Testing
- Run full test suite
- Verify feature flags

## Design Document

Full design: [2026-03-21-channel-api-abstraction-design.md](2026-03-21-channel-api-abstraction-design.md)

## Acceptance Criteria

- [ ] All channels follow the same directory structure
- [ ] `MessagingApi` is a mandatory trait
- [ ] `DocumentsApiAble`, `CalendarApiAble` are optional traits
- [ ] Tools layer depends on traits, not concrete implementations
- [ ] Tool names remain unchanged (`feishu.messages.send`, etc.)
- [ ] All feature flags work correctly
- [ ] Full test suite passes

## Benefits

| Benefit | Description |
|---------|-------------|
| **Consistency** | All channels follow identical directory structure |
| **Extensibility** | New channels (Slack/Discord) only need to implement traits |
| **Maintainability** | Shared code in `traits/`, no duplication |
| **Testability** | Traits can be mocked, tools layer independently testable |
| **Clear Dependencies** | Single dependency direction, no cycles |

## Open Questions

1. **Pagination**: Generic struct vs associated types?
2. **Platform-specific features** (Feishu cards, Telegram inline keyboards): Extension trait or separate trait?
3. **Auth abstraction**: Should we define an `AuthApi` trait?

These can be resolved during implementation.

---
