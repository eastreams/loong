# Feishu Integration Stack V1 Design

**Scope:** `loongclaw-ai/loongclaw` `alpha-test` lineage

**Problem**

LoongClaw's current Feishu/Lark support is structurally solid as an encrypted
webhook channel, but it is still only a channel adapter:

- inbound webhook verification and decrypt
- reply/send via Feishu IM APIs
- multi-account config selection
- runtime and doctor visibility for webhook serving

It does not yet provide the user-authenticated resource access surface that the
current OpenClaw Feishu plugin guide presents as the core product value:

- read Feishu docs directly
- search/read Feishu messages as the user
- inspect calendars and availability
- operate on real Feishu workspace resources under an authenticated user

The current implementation evidence shows only tenant-app token acquisition and
message send/reply endpoints in the Feishu adapter. There is no user OAuth
layer, no token lifecycle store, and no Feishu resource client layer.

**Goal**

Upgrade LoongClaw from a Feishu encrypted webhook adapter to a
`Feishu integration stack v1` that keeps transport concerns isolated while
adding:

- user OAuth and grant lifecycle
- principal-aware request context
- high-value read-only Feishu resource access
- operator-facing diagnostics for the full Feishu integration stack

## Current State Summary

What exists today:

- `crates/app/src/channel/feishu/*` handles webhook transport, payload parsing,
  reply routing, signature verification, encrypted payload handling, and runtime
  dedupe.
- `crates/app/src/config/channels.rs` supports Feishu/Lark domain selection,
  account maps, default account selection, and per-account override merging.
- `crates/daemon/src/main.rs` exposes only `feishu-send` and `feishu-serve`.
- `crates/daemon/src/doctor_cli.rs` checks Feishu channel config and webhook
  runtime, not user authorization or resource readiness.

What does not exist yet:

- Feishu user OAuth flow
- persistent grant/token storage
- user principal binding to inbound message senders
- Feishu resource clients for docs/messages/calendar
- resource-aware diagnostics and operator entry points
- topic/thread-aware Feishu session routing

## Design Principles

1. Keep transport separate from authorization and resource access.
2. Do not overload the existing Feishu channel adapter with resource APIs.
3. Do not store OAuth grants inside conversation memory.
4. Do not require full plugin parity in one pass.
5. Make every new Feishu capability diagnosable through `doctor`.
6. Add read-only resource support first; defer broad write support.

## Chosen Architecture

Introduce a new Feishu integration module in `crates/app/src/feishu/` and keep
the existing `crates/app/src/channel/feishu/` focused on transport.

### Layer 1: Channel Layer

Location:

- `crates/app/src/channel/feishu/*`

Responsibilities:

- webhook verification and decrypt
- inbound event normalization
- reply/send routing
- Feishu runtime tracker integration
- extraction of sender principal and optional thread/topic metadata

Non-goals:

- OAuth exchange
- token refresh
- resource API calls

### Layer 2: Auth Layer

New location:

- `crates/app/src/feishu/auth.rs`
- `crates/app/src/feishu/principal.rs`
- `crates/app/src/feishu/token_store.rs`

Responsibilities:

- Feishu OAuth flow bootstrap and callback handling
- grant persistence and lookup
- refresh token lifecycle
- principal binding by Feishu account and user identity
- scope validation

### Layer 3: Resource Layer

New location:

- `crates/app/src/feishu/client.rs`
- `crates/app/src/feishu/resources/docs.rs`
- `crates/app/src/feishu/resources/messages.rs`
- `crates/app/src/feishu/resources/calendar.rs`
- `crates/app/src/feishu/resources/types.rs`

Responsibilities:

- token-aware HTTP client selection
- read-only Feishu resource API wrappers
- typed request/response handling
- error normalization for operator surfaces and future tool use

## Phase 1 Scope

Phase 1 intentionally implements only the minimum high-value vertical slice.

Included:

- user OAuth state model and local token/grant store
- principal-aware Feishu request context
- read-only docs access
- read/search messages access
- read/free-busy calendar access
- Feishu auth status and diagnostics surfaces
- channel-side extraction of sender principal and optional thread/topic ids

Deferred:

- doc create/update/comment
- calendar create/update/delete
- task APIs
- base/bitable/sheets/wiki/drive full support
- broad slash-command parity
- multi-account concurrent shared-webhook supervisor

## File Layout

Add a new top-level Feishu integration module instead of growing the channel
directory into a mixed transport/business module:

- `crates/app/src/feishu/mod.rs`
- `crates/app/src/feishu/auth.rs`
- `crates/app/src/feishu/principal.rs`
- `crates/app/src/feishu/token_store.rs`
- `crates/app/src/feishu/client.rs`
- `crates/app/src/feishu/error.rs`
- `crates/app/src/feishu/resources/docs.rs`
- `crates/app/src/feishu/resources/messages.rs`
- `crates/app/src/feishu/resources/calendar.rs`
- `crates/app/src/feishu/resources/types.rs`

This keeps:

- channel concerns in `channel/feishu`
- auth and grants in `feishu/auth`
- workspace APIs in `feishu/resources`

## Authorization Model

Introduce three core types:

- `FeishuAccountBinding`
  - identifies the selected Feishu app/account from channel config
- `FeishuUserPrincipal`
  - identifies the real Feishu user, preferably via `open_id`, with account
    binding
- `FeishuGrant`
  - stores user OAuth grant material for one `(account, principal)` pair

Grant material includes:

- access token
- refresh token
- granted scopes
- expiration timestamp
- refresh timestamp

## Persistence Model

Do not reuse conversation memory storage for OAuth grants.

Add a dedicated local SQLite database for Feishu integration runtime state, for
example:

- `~/.loongclaw/feishu.sqlite3`

Recommended tables:

- `feishu_oauth_states`
- `feishu_grants`

Reasons:

- separates credential lifecycle from conversation history
- simplifies expiry/refresh/revoke semantics
- avoids polluting the memory plane with secret-bearing records
- keeps cleanup and doctor logic explicit

## Request Flow

1. Feishu webhook receives an inbound message.
2. Channel layer verifies/decrypts payload and extracts:
   - configured account identity
   - sender principal
   - conversation id
   - optional thread/topic id
   - reply target
3. Channel layer constructs a `FeishuRequestContext`.
4. If the requested operation is plain chat, normal provider flow continues.
5. If the requested operation requires Feishu resource access:
   - auth layer resolves grant for `(account, principal)`
   - missing or invalid grant returns a structured auth-required error
   - resource layer executes with a user token
6. Operator-facing diagnostics can report which layer is failing:
   - webhook/config
   - principal resolution
   - missing grant
   - expired token
   - insufficient scope
   - Feishu API error

## Operator Surfaces

### CLI

Add a focused Feishu namespace rather than many top-level commands:

- `loongclaw feishu auth start`
- `loongclaw feishu auth status`
- `loongclaw feishu auth revoke`
- `loongclaw feishu whoami`
- `loongclaw feishu read doc --url ...`
- `loongclaw feishu search messages --query ...`
- `loongclaw feishu calendar list ...`

### Doctor

Extend `doctor` with Feishu integration checks:

- app credentials present
- webhook verification configured
- principal extracted/resolvable
- user grant present
- token freshness / refreshability
- required scopes for docs/messages/calendar read

### Onboard

Keep onboarding light:

- explain that deep Feishu support requires user authorization
- point to the auth command flow
- do not embed full OAuth orchestration into the onboarding wizard

## Session and Thread Handling

The current channel model already supports optional thread ids in
`ChannelSession`, but the Feishu inbound path does not yet populate them.

Phase 1 should:

- extend Feishu inbound extraction to capture thread/topic identifiers when
  present
- use thread-aware session keys when available
- keep fallback conversation-only behavior unchanged where thread metadata is
  absent

This prepares later independent-context and multi-task behavior without
requiring a broad runtime rewrite in the first phase.

## Risks

1. Mixing OAuth logic into `channel/feishu`.
2. Reusing tenant tokens for user-scoped APIs.
3. Storing grants inside conversation memory or config files.
4. Growing `main.rs` and `doctor_cli.rs` into Feishu business-logic hubs.
5. Attempting broad write support before read-only auth and diagnostics are
   stable.

## Risk Controls

1. Enforce the `channel / auth / resource` split by file layout.
2. Use explicit auth modes in the Feishu client:
   - `Tenant`
   - `User`
3. Keep OAuth grant persistence in a dedicated SQLite store.
4. Add doctor coverage in the same phase as every new capability.
5. Ship read-only resource access first.

## Implementation Sequence

1. Create the new `crates/app/src/feishu/` skeleton and shared error/types.
2. Add token store and OAuth state/grant primitives.
3. Add user auth commands and doctor surfaces.
4. Add resource clients for docs/messages/calendar.
5. Extend channel Feishu inbound context with principal + optional thread data.
6. Connect resource reads through the new auth/resource stack.

## Acceptance Criteria For Phase 1

Phase 1 is complete when:

1. a user can complete Feishu user authorization locally
2. LoongClaw can read Feishu docs, messages, and calendar data with user scope
3. resource access failures distinguish missing grant vs expired token vs
   insufficient scope vs API failure
4. Feishu transport code remains isolated from auth/resource business logic
5. `doctor` explains Feishu integration readiness beyond webhook transport
