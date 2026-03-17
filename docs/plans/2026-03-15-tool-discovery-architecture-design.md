# Tool Discovery Architecture Design

Date: 2026-03-15
Branch: `feat/tool-discovery-architecture`
Scope: discovery-first provider tool surface for `alpha-test`
Status: Approved for implementation

## Summary

`alpha-test` still exposes the model to a broad static provider tool surface.
The current provider hot path:

- builds a full static function schema in `crates/app/src/tools/mod.rs`
- appends a full capability snapshot to the system prompt in
  `crates/app/src/provider/request_message_runtime.rs`
- accepts direct provider calls to any statically known tool in both fast-lane
  and safe-lane execution

That shape conflicts with the target product model:

- the model should begin with only a minimal core tool surface
- `tool_search` should be the path to discover non-core tools
- real execution of non-core tools should flow through one audited dispatcher
- installed external skills should not silently enlarge the provider-visible
  surface

The recommended architecture for this branch is a discovery-first hybrid:

1. Expose only `tool_search` and `tool_invoke` to provider function calling.
2. Keep all non-core tools in a runtime catalog that is searchable but not
   directly provider-callable.
3. Make `tool_search` return short tool cards plus a short-lived invoke lease,
   not full per-tool provider schemas.
4. Make `tool_invoke` validate the lease and dispatch the real tool call
   through the existing kernel-governed tool plane.
5. Shrink the system prompt capability snapshot so it only describes the
   discovery-first runtime contract.

This keeps the model-facing surface small, preserves kernel-first execution,
and creates a clean seam for later progressive exposure modes without forcing a
database or a full dynamic provider schema system into v1.

## Product Goals

- Make provider-visible tools discovery-first by default.
- Prevent direct model access to non-core tools unless they were explicitly
  discovered for the current turn.
- Reduce prompt and schema token cost for normal turns.
- Keep all real tool execution on the existing kernel-governed path.
- Preserve deterministic behavior and low operational weight.
- Keep the implementation additive and compatible with current app/kernel
  contracts.
- Leave room for future progressive exposure modes without redesigning the
  whole tool stack again.

## Non-Goals For This Slice

- No external database.
- No external vector database.
- No model-generated per-tool provider schema expansion in the first shipped
  implementation.
- No attempt to turn installed external skills into first-class provider tools.
- No new remote execution bridge or plugin sandbox in this slice.
- No removal of the existing static core tool executors themselves; only their
  provider exposure and routing model changes.

## Current State

### Provider surface is still static and broad

The provider-visible function schema is hardcoded in
`crates/app/src/tools/mod.rs` via `provider_tool_definitions()`. The request
path in `crates/app/src/provider/request_dispatch_runtime.rs` always loads that
full definition set and `crates/app/src/provider/request_payload_runtime.rs`
always emits it with `tool_choice = "auto"` when tool schema is enabled.

### The system prompt still leaks the full runtime tool surface

`crates/app/src/provider/request_message_runtime.rs` appends
`capability_snapshot_with_config(...)` into the system message. That snapshot
currently enumerates all built-in tools and can also auto-expose installed
external skills. This means the model still sees a wide tool inventory even if
the JSON function schema were narrowed later.

### Provider turns can directly request any statically known tool

`crates/app/src/conversation/turn_engine.rs` and the provider safe-lane tool
path in `crates/app/src/conversation/turn_coordinator.rs` both rely on
`is_known_tool_name()` for direct provider-emitted tool validation. Today that
means a model that already knows or guesses `file.read` or `shell.exec` can try
to call it directly.

### `tool_search` exists in `crates/spec`, not on the app/provider hot path

There is already a `tool_search` operation in `crates/spec`, but it does not
participate in the current conversation/provider runtime path. The provider hot
path lives in `crates/app`, so a discovery-first provider architecture must add
an app-native search tool rather than assuming the existing spec operation is
already in play.

### There is still a kernel-governance bypass helper

`crates/app/src/tools/mod.rs::execute_tool(...)` still falls through to direct
core execution when `kernel_ctx` is `None`. The primary provider turn path
already uses the kernel adapter directly, but keeping this helper permissive
works against the codebase's kernel-first architecture rule.

### Installed external skills are still auto-exposed by default

`ExternalSkillsRuntimePolicy::default()` currently sets
`auto_expose_installed = true`. That default conflicts with a discovery-first
contract where non-core tools and managed skills should become visible only via
explicit search or inspection flows.

## Approaches Considered

### Approach 1: Keep the current static provider tool surface

Pros:

- smallest code change
- no new routing concepts

Cons:

- large token overhead on every function-calling turn
- weak separation between core runtime tools and optional tools
- models can directly guess and call non-core tools
- installed-skill auto-exposure keeps widening the prompt surface

### Approach 2: Search first, then dynamically inject full schemas per turn

Pros:

- gives the model direct function calling for discovered tools
- good ergonomics when the provider strongly prefers explicit per-tool schemas

Cons:

- higher control-plane complexity
- requires per-turn exposure bookkeeping on request build and parser paths
- inline function parsing currently assumes a static provider schema source
- increases risk of schema drift between search results, parser expectations,
  and execution-time routing

### Approach 3: Discovery-first hybrid dispatcher

Expose only a core search/invoke pair to the provider, keep a runtime catalog
for discoverable tools, and require a short-lived lease from search before
invoke dispatches the real tool.

Pros:

- smallest provider-visible surface
- strongest default enforcement of search-first behavior
- execution still goes through one governed path
- easy to keep lightweight because catalog data can stay in memory or be
  derived from existing runtime state
- future dynamic exposure modes can still be layered on top later

Cons:

- the model calls a generic dispatcher instead of a discovered tool's native
  function name
- search results must carry enough argument hints to keep invocation usable
  without shipping full schemas

## Decision

Adopt Approach 3 as the default runtime contract for this slice.

The shipped v1 behavior should be:

- provider-visible tool schema contains only `tool_search` and `tool_invoke`
- system prompt describes only the discovery-first contract, not the full tool
  inventory
- direct provider calls to non-core tools are rejected
- `tool_search` returns short cards, not full schemas
- `tool_invoke` validates a lease and dispatches the underlying tool through
  the existing kernel-governed adapter
- installed external skills stop auto-expanding provider visibility by default

## Architecture Overview

### 1. Tool classes

The runtime tool catalog is split into two classes:

- core provider tools
  - `tool.search`
  - `tool.invoke`
- discoverable tools
  - existing built-in tools such as `claw.migrate`, `file.read`, `file.write`,
    `shell.exec`, and the managed external-skill lifecycle tools

Only core provider tools are directly valid provider-emitted tool intents.
Discoverable tools remain executable, but only behind `tool.invoke`.

### 2. Runtime catalog

Add an app-native tool catalog abstraction in `crates/app/src/tools/` that
describes:

- canonical tool id
- provider-facing function name when applicable
- exposure class
- short summary
- compact argument hint string
- required field names
- search tags

This catalog becomes the single source of truth for:

- provider core tool schema generation
- compact capability snapshot generation
- discovery search space
- direct provider-call allowlisting
- dispatcher routing to the real executor

The existing static match-based executor remains the real implementation
surface. The new catalog is a routing and disclosure layer, not a second tool
runtime.

### 3. `tool_search`

`tool_search` is promoted into the app/provider runtime as a first-class core
tool. It should:

- accept a natural-language `query`
- return only discoverable tools
- rank results deterministically using local metadata
- return compact cards with:
  - `tool_id`
  - `summary`
  - `argument_hint`
  - `required_fields`
  - `tags`
  - `why`
  - `lease`

The lease is an invoke-scoped token bound to:

- target tool id
- expiration time
- current catalog/schema digest
- process-local secret

The token is short-lived and self-validating so v1 does not require a database.

### 4. `tool_invoke`

`tool_invoke` is the only provider-visible path to non-core tool execution.
It should:

- accept `tool_id`, `lease`, and `arguments`
- validate the lease and tool id match
- reject expired or tampered leases
- dispatch only to discoverable tools
- never dispatch back into the provider-visible core pair

The dispatch itself still uses the current app executor and kernel adapter
stack. The model-facing function name changes, but the underlying governance
path does not.

### 5. Provider exposure modes

This slice ships only the dispatcher-only mode, but the architecture should
leave space for two future modes:

- `dispatcher_only`
  - current slice
- `ephemeral_stub`
  - search returns a temporary handle and a narrow callable stub may be exposed
    on the next round
- `ephemeral_full`
  - search results can escalate to full per-tool provider schemas for a later
    round when justified

Those later modes should reuse the same catalog and lease model. The source of
truth remains the catalog plus the dispatcher, not ad hoc schema generation.

### 6. Prompt surface minimization

The system prompt should stop enumerating the full tool surface. Instead it
should describe the contract:

- `tool_search` discovers task-relevant tools
- `tool_invoke` executes a previously discovered tool with a valid lease
- non-core tools are intentionally hidden until discovered

This cuts prompt tokens and prevents prompt-level leakage from bypassing the
discovery-first model.

### 7. External skills behavior

Installed external skills should no longer auto-appear in the provider prompt
surface by default. Their runtime tools remain discoverable through
`tool_search`, and explicit managed-skill inspection/invocation continues to
work through the dispatcher path.

## User-Facing Runtime Model

### Normal turn

The model sees only:

- `tool_search`
- `tool_invoke`

The system prompt tells the model to search before invoking non-core tools.

### Discovery turn

The model calls `tool_search("read a file from the repo")`.

The runtime returns a short card such as:

- `tool_id = "file.read"`
- `summary = "Read file contents"`
- `argument_hint = "path:string,max_bytes?:integer"`
- `required_fields = ["path"]`
- `lease = "..."`

### Execution turn

The model calls `tool_invoke` with:

- the returned `tool_id`
- the returned `lease`
- an `arguments` object for the real tool payload

`tool_invoke` validates the lease and dispatches the underlying tool through
the existing governed executor.

### Failure behavior

If the model tries to call `file.read` directly as a provider tool:

- provider parsing may still recover the intent
- execution rejects it as not provider-exposed
- the runtime can steer the model back toward `tool_search`

If the model reuses an expired or tampered lease:

- `tool_invoke` fails closed
- the reply explains the model must re-run `tool_search`

## Security, Stability, and Lightweight Constraints

### Security

- direct provider calls are limited to the core pair
- all real tool execution still goes through the kernel-governed tool path
- `tool.invoke` derives required kernel capabilities from the effective target
  tool request instead of stopping at `InvokeTool`
  - `file.read` requires `InvokeTool + FilesystemRead`
  - `file.write` requires `InvokeTool + FilesystemWrite`
  - writeful `claw.migrate` modes require `FilesystemWrite` in addition to read
- leases are scoped to one tool id and expire quickly
- file-root and migration-root escapes surface as explicit `policy_denied:`
  tool-plane failures so retry logic stays fail-closed
- shell execution keeps a default-deny posture unless an explicit allowlist is
  configured
- installed-skill auto-exposure defaults off
- the permissive `execute_tool(..., None)` helper should be closed in this
  slice so kernel-first is true in code, not only in docs

### Stability

- catalog metadata is static/derived from existing runtime configuration
- provider schema shape becomes much smaller and more stable
- inline function parser no longer needs to track the full tool universe for
  the provider surface
- no new durable state is required for v1

### Lightweight operation

Do not add a database in v1.

The right lightweight default is:

- static or derived metadata catalog
- in-memory ranking
- stateless signed lease tokens
- existing on-disk managed-skill index reused as an input when needed

If future scale or ranking quality requires acceleration, the next step should
be a local derived cache such as SQLite/FTS. It should remain an optimization
layer, not the source of truth.

## Validation Strategy

### New behavior tests

- provider tool definitions expose only `tool_search` and `tool_invoke`
- compact capability snapshot no longer leaks discoverable tools
- `tool_search` returns only discoverable tools and returns leases
- `tool_invoke` dispatches a discovered tool successfully
- `tool_invoke` rejects tampered or expired leases
- `tool.invoke` enforces the effective tool capability boundary for file and
  import tools
- file-root escape paths classify as policy denial instead of retryable tool
  execution errors
- shell runtime defaults remain default-deny when no allowlist is configured
- direct provider validation rejects non-core tools
- installed external skills are not auto-exposed by default
- `execute_tool(..., None)` fails closed instead of bypassing the kernel

### Existing regression coverage

- direct tool execution through the kernel adapter still works
- safe-lane and fast-lane both execute `tool_invoke`
- provider request fallback for tool-schema-unsupported models still behaves
  correctly

## Follow-On Work

After this slice is stable, the next architectural options are:

1. add optional progressive exposure modes (`ephemeral_stub`,
   `ephemeral_full`)
2. improve search ranking with derived local indexing
3. add richer tool-card hints for complex tools without shipping full schemas
4. add explicit runtime metrics around search-hit rate, lease failures, and
   dispatcher-only success rate
