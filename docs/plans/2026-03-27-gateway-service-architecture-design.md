# Gateway Service Architecture Design

**Problem**

LoongClaw already contains the core pieces of a future gateway-oriented agent
runtime, but those pieces are still expressed as separate command surfaces,
channel-specific listeners, and CLI-oriented operator views instead of one
explicit service boundary.

That gap matters because the intended product direction now clearly extends
beyond a terminal-first assistant process. The planned direction includes:

- remote browser and mobile pairing
- always-on service installation and background runtime ownership
- route mounting, external APIs, auth tokens, and port binding
- richer long-lived channel runtimes such as Discord, Slack, and WhatsApp
- dashboard, status, logs, and observability as first-class operator surfaces
- full lifecycle separation between interactive CLI sessions and the service
  that owns channels, routes, and long-lived runtime state

Without an explicit gateway layer, those capabilities will be added piecemeal
across `multi-channel-serve`, Web UI, browser companion, webhook channels, ACP
operator commands, and future channel runtimes. That would create a distributed
service boundary with unclear ownership and rising integration debt.

## Decision Summary

- LoongClaw already has gateway substrate in the daemon supervisor, channel
  ingress normalization, protocol transport, ACP control-plane surfaces, and
  daemon CLI read models.
- LoongClaw does not yet have an explicit gateway service contract, gateway
  owner state, route-mount layer, or unified auth boundary.
- `multi-channel-serve` should be treated as the first runtime-owner precursor,
  not as the long-term product noun.
- Gateway is required for the accepted product direction, but the early slices
  can remain localhost-only by default, security-governed, and
  operator-governed while the broader service contract matures.

## Current Architecture Evidence

The repository already contains most of the technical substrate needed for a
gateway service. The problem is not missing fundamentals; it is missing
composition and explicit ownership.

### 1. A runtime owner already exists, but only as a CLI command

`multi-channel-serve` already behaves like a narrow runtime owner:

- it supervises enabled runtime-backed channels in one process
- it keeps a foreground concurrent CLI host
- it tracks lifecycle state and coordinated shutdown
- it fails the whole owner when one required background surface fails

Evidence:

- `README.md`
- `docs/product-specs/channel-setup.md`
- `crates/daemon/src/supervisor.rs`
- `crates/app/src/chat.rs`

This is the strongest proof that LoongClaw is not starting from zero. The
repository already has a service-host seed, but the surface is still expressed
as one command (`multi-channel-serve`) instead of a first-class gateway service
contract.

### 2. Channel ingress is already normalized into typed runtime context

Inbound channel messages do not go straight into provider glue. They are
normalized into:

- `ConversationSessionAddress`
- `ConversationIngressContext`
- ACP turn provenance

before entering the conversation turn coordinator.

Evidence:

- `crates/app/src/channel/mod.rs`
- `crates/app/src/conversation/turn_coordinator.rs`
- `docs/design-docs/acp-acpx-preembed.md`

That is exactly the kind of normalization a gateway service should own at the
system boundary.

### 3. ACP already exists as a separate control plane

LoongClaw already distinguishes:

- conversation/context assembly
- ACP runtime backends
- ACP session manager and binding store
- ACP status, dispatch, and observability surfaces

Evidence:

- `crates/app/src/acp/mod.rs`
- `crates/app/src/acp/runtime.rs`
- `crates/app/src/acp/manager.rs`
- `crates/daemon/src/lib.rs`
- `docs/design-docs/acp-acpx-preembed.md`

This is important because the gateway should not absorb ACP. ACP is already the
right internal control-plane seam. The gateway should host and expose it, not
replace it.

### 4. Operator-facing read models already exist, but only as CLI payloads

The daemon crate already exposes structured JSON/text operator surfaces for:

- channel inventory and status
- ACP status
- ACP dispatch evaluation
- ACP observability
- runtime snapshot

Evidence:

- `crates/daemon/src/lib.rs`

These functions are already close to future gateway API handlers. The missing
step is to extract them into service read models instead of leaving them as
command-local `println!` flows.

### 5. Protocol and transport primitives already exist

The `protocol` crate already provides:

- typed frame envelopes
- route validation
- route authorization
- JSON-line transport
- linked in-memory channel transport

Evidence:

- `crates/protocol/src/lib.rs`
- `docs/design-docs/index.md`
- `docs/design-docs/layered-kernel-design.md`

That gives LoongClaw an existing substrate for:

- gateway-internal control transport
- gateway-to-node transport
- companion-runtime transport
- future local or remote control APIs

### 6. The browser companion already wants a managed runtime relationship

The browser companion direction already assumes:

- structured protocol instead of raw shell passthrough
- LoongClaw-issued session identifiers
- bounded execution
- runtime availability gates
- integration with onboarding, doctor, and runtime visibility

Evidence:

- `docs/product-specs/browser-automation-companion.md`
- `docs/ROADMAP.md`

That is much closer to a future gateway-managed node capability than to a pure
standalone shell helper.

## Current Gaps

Despite the strong substrate, LoongClaw still lacks the explicit service layer
needed for the target product direction.

### 1. No explicit gateway service object

There is no `gateway run/start/stop/status/logs` contract. Runtime ownership is
still framed as:

- `chat`
- per-channel `*-serve`
- `multi-channel-serve`

This keeps process ownership and operator expectations fragmented.

### 2. No unified network and auth boundary

There is no service-level contract for:

- bind address ownership
- port ownership
- local admin token
- pairing token
- node registration
- external API authorization

Those concerns are not just implementation details. They are product and
security boundaries.

### 3. Webhook channels still self-own listeners

Webhook-based channels such as Feishu currently bind listeners directly inside
channel modules. That works for isolated commands, but it scales poorly once
one gateway process should own:

- multiple mounted routes
- shared auth and health surfaces
- future API endpoints
- Web UI endpoints
- node and pairing flows

### 4. No cross-process gateway owner state

Channel runtime state already persists ownership and liveness information, but
there is no equivalent persisted gateway owner state containing:

- pid
- bind
- port
- runtime version
- start time
- active mode
- token file location
- socket or event-stream metadata

### 5. No explicit node model

Remote browser, mobile pairing, and managed sidecar runtimes require more than
channel supervision. They require a stable model for:

- node identity
- capability advertisement
- pairing and trust establishment
- heartbeat and health
- route selection
- session-scoped capability ownership

### 6. Current product docs are still scoped narrower than the intended target

Some current docs still describe Web UI and current concurrent runtime slices in
ways that are too narrow for the intended service/gateway direction. That
creates strategic drift between implementation decisions and product goals.

## External Calibration

OpenClaw is the clearest external comparison point because it already treats the
gateway as the durable runtime host for dashboard, long-lived channel runtime,
browser-facing operations, and node attachment.

The main architectural reasons claw-style systems add an explicit gateway are:

1. one always-on service host for multiple clients and channels
2. one explicit bind, route-mount, and auth boundary
3. one operator-visible dashboard, status, and log surface
4. one attachment model for browser, mobile, and other paired nodes
5. one lifecycle owner that decouples service uptime from any attached CLI

LoongClaw now faces the same product pressures. The lesson to borrow is unified
service ownership, not an automatic jump to hosted multi-tenant semantics.

## Security Default, Not Product Retreat

The current localhost-only bind policy should be understood as a security
default, not as a strategic refusal of broader gateway capability.

More specifically, the current policy means:

- the gateway should not be publicly exposed by default
- widening bind scope requires an explicit auth, lifecycle, and observability
  contract
- current Web UI and operator flows should attach through the daemon-owned
  gateway surface instead of inventing a second runtime

It does not mean:

- remote browser or mobile pairing is out of scope
- always-on service ownership is out of scope
- stable bind and port ownership are out of scope
- richer long-lived channel runtimes are out of scope

The architectural mistake to avoid is treating "localhost-only by default" as a
product noun. It is a rollout and security posture for the current slice.

## Design Goals

1. Add an explicit gateway service boundary above existing daemon/operator
   surfaces.
2. Reuse existing conversation, provider, tool, memory, and ACP semantics
   instead of creating a second assistant runtime.
3. Keep governance in the kernel and app execution layers, not in the gateway
   transport layer.
4. Centralize long-lived service ownership, route mounting, and API/auth
   boundaries.
5. Make dashboard, Web UI, CLI, and future mobile clients consumers of the same
   gateway service core.
6. Keep the first gateway slices single-runtime and operator-governed rather
   than prematurely widening to full hosted multi-tenant semantics.
7. Preserve additive evolution so existing `chat`, `doctor`, `channels`, and
   `*-serve` surfaces can remain compatible while the gateway becomes the
   preferred owner.

## Non-goals

- Do not move kernel policy, audit, or capability logic into the gateway.
- Do not merge ACP into the gateway.
- Do not make Web UI a separate runtime.
- Do not require all channels to become HTTP route mounts; long-connection and
  polling runtimes should remain supervised tasks where appropriate.
- Do not force immediate public-internet exposure as part of the first gateway
  slice.
- Do not treat the current localhost-only default as evidence that future
  service-mode or remote attachment work is undesired.
- Do not commit this slice to a hosted multi-tenant SaaS contract.

## Core Idea

LoongClaw should introduce a **gateway service layer** inside `crates/daemon`
that owns service lifecycle, route mounting, auth, operator APIs, and runtime
supervision while delegating execution semantics to existing `app`, `kernel`,
and `protocol` layers.

This means the future model becomes:

1. `daemon/gateway` owns the service boundary and operator/network entrypoints.
2. `supervisor` becomes one runtime mode inside gateway service ownership.
3. `app/channel` continues to normalize inbound channel events and execute
   channel adapters.
4. `conversation` continues to own turn execution semantics.
5. `acp` continues to own runtime control-plane semantics.
6. `protocol` becomes the reusable transport substrate for gateway-internal and
   gateway-external control paths.

## Proposed Target Architecture

### Layer A: Gateway Service Core

Create a gateway service core inside `crates/daemon/src/gateway/` with types
such as:

- `GatewayConfig`
- `GatewayRuntime`
- `GatewayOwnerState`
- `GatewayMode`
- `GatewayReadModels`
- `GatewayAuthPolicy`

Responsibilities:

- load and validate service config
- acquire and persist gateway owner state
- initialize auth material
- initialize runtime supervision
- construct read models used by CLI, HTTP APIs, and dashboard/Web UI

### Layer B: Gateway Runtime Ownership

Absorb the current `multi-channel-serve` supervisor into gateway runtime
ownership.

Key principle:

- `multi-channel-serve` is not the long-term product noun
- it is one gateway runtime mode or one compatibility alias

The gateway runtime owner should eventually own:

- background runtime-backed channel surfaces
- the foreground or attached CLI client mode
- clean shutdown and signal handling
- runtime restart policy
- future detached/background mode

### Layer C: Gateway Route Mounting

Introduce a gateway route-mount model that distinguishes:

- mounted HTTP callback routes
- background supervised channel runtimes
- local service APIs
- Web UI routes
- node or pairing routes

This allows webhook channels to become descriptors that register route handlers
into the gateway instead of directly binding listeners in channel modules.

### Layer D: Gateway Operator API

Promote existing CLI JSON payload builders into typed service read models, then
reuse them across:

- CLI output
- HTTP JSON responses
- dashboard/Web UI
- future mobile clients

Initial read-model candidates already exist in the daemon crate:

- channel inventory
- ACP status
- ACP dispatch evaluation
- ACP observability
- runtime snapshot

### Layer E: Gateway Event Stream

Add a gateway event stream for:

- runtime lifecycle updates
- channel surface state transitions
- ACP runtime events
- operator-facing warnings and failures
- future dashboard live updates

The first event stream does not need to invent a new semantic model. It should
reuse existing persisted or typed event payloads where possible.

### Layer F: Gateway Node Model

Introduce an explicit node model for future sidecars and paired clients.

Suggested early node categories:

- `browser_companion`
- `mobile_client`
- future external tool relay or device runtime

Each node should carry:

- stable `node_id`
- auth and pairing state
- capability summary
- liveness and heartbeat
- transport metadata
- session or route ownership metadata when relevant

This is the missing seam for remote/browser/mobile pairing.

## Boundary Rules

### 1. Gateway does not replace kernel governance

All security-sensitive execution must continue to route through kernel-governed
or app-governed paths. The gateway can authenticate transport clients and
authorize route categories, but it must not become a second policy engine for
tool or memory semantics.

### 2. Gateway does not replace ACP

ACP remains the internal runtime control plane for:

- backend selection
- session binding
- dispatch reasoning
- status and observability
- runtime events

The gateway should expose ACP, not absorb it.

### 3. Web UI is a gateway client, not a runtime fork

Web UI should remain a consumer of gateway service read models and APIs. It
must not gain a separate turn pipeline, memory semantics, or hidden control
paths.

### 4. Channel adapters stay in `app`

Channel config, transport normalization, send behavior, and execution semantics
should remain in `crates/app/src/channel/*`.

The gateway only owns:

- service listener lifecycle
- route mounting
- surface supervision
- operator APIs

### 5. Protocol stays reusable

The protocol crate should remain generic enough to support:

- gateway internal control transport
- companion runtime transport
- future local or remote control channels

without hard-coding gateway business semantics into protocol types.

### 6. Default bind policy is a security boundary

The gateway can remain localhost-only by default for the current slice without
turning that posture into the long-term product boundary.

That means:

- current slices should prefer loopback-only bind by default
- future service mode may introduce stable or configurable bind ownership
- broader exposure must follow explicit auth, token, operator-control, and
  observability work

## Proposed Command Surface

Introduce a new top-level command family:

- `loongclaw gateway run`
- `loongclaw gateway start`
- `loongclaw gateway stop`
- `loongclaw gateway status`
- `loongclaw gateway logs`

Compatibility direction:

- keep `multi-channel-serve` initially as an alias or narrow wrapper
- keep `chat` as an attached interactive client mode
- keep `*-serve` while route-mount and gateway ownership are still migrating

## Proposed Gateway API Surface

The first HTTP/API surface should stay intentionally small and reuse existing
read models:

- `GET /v1/status`
- `GET /v1/channels`
- `GET /v1/acp/status`
- `GET /v1/acp/observability`
- `GET /v1/acp/dispatch`
- `GET /v1/runtime/snapshot`
- `GET /v1/events`
- `POST /v1/gateway/shutdown`
- `POST /v1/pairing/start`
- `POST /v1/pairing/complete`

The exact route names are less important than the rule:

- service handlers must return typed service models
- CLI and HTTP should render from the same typed state

## Auth Model

The gateway should use three explicit auth lanes.

### 1. Local admin auth

Used by:

- local CLI client commands
- local Web UI
- local operator tools

Backed by:

- token file or local secret material
- strict localhost or explicit bind policy

### 2. Pairing auth

Used for:

- browser-based pairing
- mobile pairing
- one-time device registration

Backed by:

- short-lived pairing token
- explicit approval or operator confirmation

### 3. Node auth

Used for:

- browser companion
- future managed remote nodes

Backed by:

- node registration record
- renewable node token
- capability-scoped authorization

## Persistence Model

The gateway should introduce a top-level persisted state directory parallel to
current channel runtime state.

Suggested artifacts:

- `gateway/owner.json`
- `gateway/token`
- `gateway/events/`
- `gateway/nodes/`
- `gateway/routes/`

Reuse existing persisted sources where possible:

- channel runtime state files remain the truth for per-channel runtime liveness
- ACP store remains the truth for ACP session state
- audit JSONL remains the truth for kernel audit evidence

The gateway should aggregate these sources, not replace them.

## Migration Strategy

### Phase 1: Service core extraction

- extract daemon CLI JSON payload builders into typed gateway service models
- introduce `gateway` module and owner state
- add `gateway run` and `gateway status`

### Phase 2: Runtime owner promotion

- make current supervisor a gateway-owned runtime mode
- keep `multi-channel-serve` as a compatibility alias
- separate attached CLI mode from background service mode

### Phase 3: Route mounting and local HTTP API

- centralize service bind ownership
- mount Web UI and service control routes in one gateway host
- convert webhook channels to mount handlers instead of self-binding listeners

### Phase 4: Node and pairing

- introduce browser companion as the first managed node
- add pairing flows and capability advertisement
- add event stream support for dashboards and paired clients

### Phase 5: Service install and richer channels

- add launchd/systemd/Windows-service ownership
- add restart/backoff policy
- land gateway-native runtimes such as Discord and Slack on top of the new
  service layer

## Risks

### 1. Gateway and ACP role confusion

If gateway APIs start re-implementing ACP semantics, the repository will gain
two overlapping control planes.

### 2. Web UI runtime drift

If Web UI gains direct turn execution paths outside the service core, LoongClaw
will reintroduce the "second runtime" problem under a different name.

### 3. Channel adapter sprawl

If each future channel continues to own bind/listener/auth/lifecycle behavior
itself, the gateway layer will arrive too late and be forced to wrap many
incompatible implementations.

### 4. Direct-binding regressions

Gateway production paths must not fall back to weak direct execution semantics
for runtime mutation or remote control paths.

### 5. Scope explosion

The gateway layer is strategically necessary, but it should still land in thin
slices. The first milestone should be explicit service ownership, not full
distributed systems scope.

## Open Questions

1. Should the first gateway slice stay strictly single-runtime per machine, or
   should it support named profiles or multiple instances immediately?
2. Should event streaming use SSE first, WebSocket first, or both?
3. Should local CLI attach to the gateway over HTTP, JSON-line transport, or a
   purely in-process service interface in the first slice?
4. Should the first browser companion node transport reuse the current managed
   preview path or move immediately to a gateway-owned session transport?
5. How much public-remote exposure should be allowed before stronger exposure
   policy and docs land?

## Decision

LoongClaw should formally adopt a gateway service architecture in the daemon
layer and treat the current supervisor, operator JSON surfaces, protocol crate,
channel ingress normalization, and ACP control plane as the substrate for that
gateway rather than continuing to grow those responsibilities as disconnected
product slices.
