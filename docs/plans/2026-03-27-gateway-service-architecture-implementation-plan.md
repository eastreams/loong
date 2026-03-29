# Gateway Service Architecture Implementation Plan

> Execution note: this plan assumes the gateway direction is accepted as the
> intended service architecture for LoongClaw. Implement it in thin vertical
> slices that keep CLI compatibility while progressively promoting gateway-owned
> service semantics.

**Goal:** Introduce an explicit gateway service layer in `crates/daemon` that
owns lifecycle, route mounting, auth, operator APIs, and runtime supervision
without replacing existing conversation, tool, memory, provider, or ACP
semantics.

**Architecture:** Keep service ownership in `crates/daemon`, channel execution
and ingress normalization in `crates/app`, governance in `crates/kernel`, and
transport contracts in `crates/protocol`. Reuse current supervisor state,
channel inventory, ACP status/dispatch/observability, runtime snapshots, and
browser companion readiness work as the substrate of the gateway service rather
than building a second runtime.

**Tech Stack:** Rust, Tokio, Axum, existing LoongClaw daemon/app/protocol
crates, JSON payload builders, persisted runtime state files, cargo test, cargo
clippy, `task verify`

---

## Task 1: Land the architecture contract

**Files:**
- Create: `docs/plans/2026-03-27-gateway-service-architecture-design.md`
- Create: `docs/plans/2026-03-27-gateway-service-architecture-implementation-plan.md`
- Modify: `README.md`
- Modify: `docs/ROADMAP.md`
- Modify: `docs/product-specs/browser-automation-companion.md`
- Modify: `docs/product-specs/web-ui.md`
- Modify: `docs/product-specs/channel-setup.md`

**Step 1: Update product and roadmap wording to match the accepted direction**

- replace wording that frames gateway ownership as an out-of-scope concept for
  the long-term architecture
- keep current slice boundaries accurate, but describe them as incremental
  delivery steps toward a gateway service layer
- update Web UI wording so it remains "not a second runtime" while no longer
  implying that the local-only posture is the strategic endpoint
- update browser companion wording so it remains optional while clearly pointing
  toward a future gateway-managed node model

**Step 2: Verify documentation consistency**

Run:

```bash
LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh
```

Expected: PASS once repository-wide release-doc governance gaps are already
clean. If this command fails only on pre-existing release artifacts outside the
gateway slice, record that blocker explicitly and continue validating the
touched gateway docs for internal consistency.

## Task 2: Extract daemon operator payloads into gateway service read models

**Files:**
- Create: `crates/daemon/src/gateway/mod.rs`
- Create: `crates/daemon/src/gateway/read_models.rs`
- Modify: `crates/daemon/src/lib.rs`
- Test: `crates/daemon/tests/integration/*`

**Step 1: Write failing tests for typed gateway read models**

Add tests with a `gateway_read_model_` prefix that prove:

- channel inventory can be produced without printing
- ACP status can be produced as a typed struct
- ACP observability can be produced as a typed struct
- ACP dispatch can be produced as a typed struct
- runtime snapshot can embed the same typed inventory and ACP state

**Step 2: Run the targeted tests and confirm failure**

Run:

```bash
cargo test -p loongclaw-daemon gateway_read_model_ -- --test-threads=1
```

Expected: FAIL because no shared gateway read-model layer exists yet.

**Step 3: Implement the read-model layer**

Move CLI-oriented payload construction behind gateway-owned types such as:

- `GatewayChannelInventorySnapshot`
- `GatewayAcpStatusSnapshot`
- `GatewayAcpObservabilitySnapshot`
- `GatewayAcpDispatchSnapshot`
- `GatewayRuntimeSnapshot`

Update existing CLI surfaces to render from those types instead of constructing
JSON inline inside command handlers.

**Step 4: Re-run the same tests and confirm PASS**

## Task 3: Introduce gateway owner state and lifecycle core

**Files:**
- Create: `crates/daemon/src/gateway/service.rs`
- Create: `crates/daemon/src/gateway/state.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Test: `crates/daemon/src/gateway/*.rs`

**Step 1: Add the gateway CLI contract**

Add a new top-level command family:

- `gateway run`
- `gateway status`
- `gateway stop`

The first slice can keep `start` and `logs` deferred if needed, but `run`,
`status`, and `stop` should be explicit.

**Step 2: Persist gateway owner state**

Add a persisted owner record containing at least:

- pid
- bind
- port
- runtime version
- start time
- mode
- token path when auth is enabled

Use a gateway-owned state directory rather than overloading per-channel runtime
state files.

**Step 3: Verify lifecycle behavior**

Add tests with a `gateway_owner_state_` prefix that prove:

- only one gateway owner can claim the service slot
- stale gateway owner state can be reclaimed safely
- gateway status reports stopped/running/failed deterministically

Run:

```bash
cargo test -p loongclaw-daemon gateway_owner_state_ -- --test-threads=1
```

Expected: PASS.

## Task 4: Promote the current supervisor into a gateway runtime mode

**Files:**
- Modify: `crates/daemon/src/supervisor.rs`
- Create: `crates/daemon/src/gateway/runtime.rs`
- Modify: `crates/app/src/chat.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/main.rs`
- Test: `crates/daemon/tests/integration/multi_channel_serve_cli.rs`

**Step 1: Reframe `multi-channel-serve` as a compatibility wrapper**

Keep `multi-channel-serve` working, but make it invoke a gateway runtime mode
instead of remaining the long-term runtime-owner noun.

**Step 2: Add gateway runtime modes**

Suggested early modes:

- `AttachedCliWithBackgroundChannels`
- `DetachedService`

The first detached slice may still supervise the same runtime-backed channels as
the current supervisor.

**Step 3: Preserve concurrent CLI host behavior**

Reuse:

- `ConcurrentCliHostOptions`
- `ConcurrentCliShutdown`
- existing foreground host loop

but make CLI attach a mode of the gateway runtime rather than the sole
definition of it.

**Step 4: Verify compatibility**

Add tests with a `gateway_runtime_mode_` prefix that prove:

- `multi-channel-serve` remains functional
- `gateway run --attach-cli` uses the same session isolation rules
- background failure still tears down the runtime cleanly

## Task 5: Add a local HTTP control plane and event stream

**Files:**
- Create: `crates/daemon/src/gateway/api_http.rs`
- Create: `crates/daemon/src/gateway/api_events.rs`
- Modify: `crates/daemon/src/gateway/mod.rs`
- Test: `crates/daemon/tests/integration/*`

**Step 1: Start with a narrow API surface**

Initial routes should be read-only or tightly scoped:

- `GET /v1/status`
- `GET /v1/channels`
- `GET /v1/acp/status`
- `GET /v1/acp/observability`
- `GET /v1/acp/dispatch`
- `GET /v1/runtime/snapshot`
- `GET /v1/events`

These should render the same gateway read models as the CLI.

**Step 2: Add a first event stream**

Prefer one event-stream surface first, such as SSE, for:

- runtime lifecycle updates
- channel state transitions
- ACP runtime events
- gateway warnings and failures

**Step 3: Add tests**

Add tests with a `gateway_http_api_` prefix that prove:

- routes return stable JSON
- auth is required on mutation-sensitive routes
- event stream emits typed payloads

Run:

```bash
cargo test -p loongclaw-daemon gateway_http_api_ -- --test-threads=1
```

Expected: PASS.

## Task 6: Centralize route mounting for webhook channels

**Files:**
- Create: `crates/daemon/src/gateway/routes/`
- Modify: `crates/app/src/channel/feishu/mod.rs`
- Modify: `crates/app/src/channel/sdk.rs`
- Modify: `crates/app/src/channel/registry.rs`
- Test: `crates/app/src/channel/feishu/*`
- Test: `crates/daemon/tests/integration/*`

**Step 1: Define a channel route-mount contract**

Introduce a small app-owned or gateway-facing descriptor for callback routes so
webhook channels can expose:

- route path
- HTTP method
- state builder
- handler entrypoint

without owning the listener themselves.

**Step 2: Migrate Feishu webhook first**

Feishu is the best first candidate because it already binds `axum` routes
directly in the channel module today.

Goal:

- gateway owns the listener
- Feishu exports a mountable handler contract
- channel code keeps auth, payload parsing, and reply behavior

**Step 3: Verify behavior**

Add tests with a `gateway_route_mount_` prefix that prove:

- the mounted Feishu route behaves the same as before
- graceful shutdown still works
- bind ownership has moved to the gateway

## Task 7: Introduce gateway auth and pairing primitives

**Files:**
- Create: `crates/daemon/src/gateway/auth.rs`
- Create: `crates/daemon/src/gateway/pairing.rs`
- Modify: `crates/daemon/src/gateway/state.rs`
- Test: `crates/daemon/src/gateway/*.rs`

**Step 1: Add local admin auth**

Implement a local admin token or equivalent secret material for:

- CLI operator calls into the gateway
- local Web UI control surfaces
- future local automation scripts

**Step 2: Add pairing tokens**

Introduce short-lived pairing sessions for:

- browser-based pairing
- mobile device registration
- future remote control clients

**Step 3: Add tests**

Add tests with a `gateway_auth_` prefix that prove:

- missing or invalid token fails closed
- pairing tokens expire deterministically
- local admin and pairing auth lanes stay distinct

## Task 8: Promote the browser companion to a gateway-managed node

**Files:**
- Create: `crates/daemon/src/gateway/nodes.rs`
- Modify: `crates/daemon/src/browser_preview.rs`
- Modify: `crates/app/src/tools/*`
- Modify: `docs/product-specs/browser-automation-companion.md`
- Test: `crates/daemon/tests/integration/*`

**Step 1: Introduce a node record**

Add a first-class node model carrying:

- `node_id`
- node type
- health
- capabilities
- auth state
- last heartbeat
- transport metadata

**Step 2: Make the browser companion the first managed node**

The browser companion already wants:

- structured protocol
- session IDs
- bounded execution
- runtime visibility

Use that as the first node integration instead of inventing a different pairing
shape later.

**Step 3: Verify runtime-visible behavior**

Add tests with a `gateway_node_browser_companion_` prefix that prove:

- gateway can register and track the browser companion node
- runtime-visible browser companion tools still depend on real readiness
- companion sessions remain bounded and fail closed

## Task 9: Add service installation and detached ownership

**Files:**
- Create: `crates/daemon/src/gateway/install.rs`
- Modify: `scripts/install.sh`
- Modify: `scripts/install.ps1`
- Modify: release docs and onboarding docs
- Test: install and integration tests

**Step 1: Add detached service management**

Introduce detached service flows for supported platforms:

- macOS `launchd`
- Linux `systemd`
- Windows service or the repo's chosen equivalent

**Step 2: Keep platform support explicit**

Fail closed on unsupported platforms or incomplete installs. Do not silently
pretend the service installed.

**Step 3: Verify**

Add tests and scripts that validate:

- install metadata generation
- status detection
- clean stop and uninstall

## Task 10: Migrate richer channels onto the gateway service layer

**Files:**
- Modify: `crates/app/src/channel/sdk.rs`
- Modify: `crates/app/src/channel/registry.rs`
- Modify: future channel adapters and runtime builders
- Test: per-channel integration suites

**Step 1: Use the gateway layer as the prerequisite for gateway-native channels**

Channels such as Discord and Slack should land only after the gateway provides:

- route/session ownership
- unified service lifecycle
- health and status surfaces
- auth and route mounting where needed

**Step 2: Keep the registry-first model**

Continue deriving runtime-backed surfaces from registry and SDK metadata instead
of daemon-local special cases.

**Step 3: Verify each new runtime-backed channel**

For each channel, test:

- catalog/inventory integration
- runtime ownership and liveness
- route or socket lifecycle
- status/read-model integration
- gateway shutdown behavior

## Verification Standard

Before every commit in gateway implementation slices, run:

```bash
task verify
```

For narrower slices, also run targeted tests with shared prefixes such as:

- `gateway_read_model_`
- `gateway_owner_state_`
- `gateway_runtime_mode_`
- `gateway_http_api_`
- `gateway_route_mount_`
- `gateway_auth_`
- `gateway_node_browser_companion_`

## Delivery Order

Recommended execution order:

1. architecture contract and doc alignment
2. read-model extraction
3. gateway owner state and lifecycle core
4. supervisor promotion into gateway runtime mode
5. local HTTP API and event stream
6. route mounting for webhook channels
7. auth and pairing
8. browser companion as first node
9. service installation
10. gateway-native channel rollout

## Expected Outcome

After these slices, LoongClaw will no longer express service ownership as a
collection of loosely related CLI commands. It will have an explicit gateway
service layer that:

- owns lifecycle and supervision
- exposes reusable read models and APIs
- centralizes route mounting and auth
- preserves ACP and channel boundaries
- gives Web UI, browser companion, and future mobile clients one coherent
  runtime host to attach to
