# Local Product Control Plane

## Purpose

LoongClaw already has the hard parts of a kernel-first system:

- a governed execution boundary in the kernel
- a real ACP control plane for backend session lifecycle and routing
- a durable `SessionRepository` for session lineage, events, approvals, and outcomes
- operator-facing CLI surfaces such as `onboard`, `doctor`, `acp-status`, and `list-acp-sessions`

What it does not yet have is one written contract for how those pieces become a
coherent local product platform.

That gap is now risky because:

- `#217` tracks an always-on operator/runtime surface
- `#296` tracks a minimal HTTP gateway
- `#403` tracks a browser-first launcher and Web UI path

Without an explicit product control plane contract, those efforts can drift into
one of two bad outcomes:

- a giant gateway that becomes a second authority center above the kernel
- separate browser, CLI, and runtime pathways that each invent their own
  session and status semantics

This document defines the smaller LoongClaw-native path.

It intentionally keeps the public repository artifact scoped to the
implementation-facing contract. The fuller comparative analysis, provenance
trail, and extended references for this design are archived in
`lc-knowledge-base`.

## Design Goals

1. Keep the kernel as the authority for governance, policy, approvals, and
   audit.
2. Keep ACP as an internal runtime control plane rather than turning it into
   the product-facing gateway by accident.
3. Make the local product layer explicitly localhost-first and operator-owned.
4. Reuse one canonical session plane across CLI, HTTP, and future Web UI
   surfaces.
5. Let `onboard`, `doctor`, `status`, approvals, and turn submission converge
   through shared operations instead of surface-specific logic.

## Non-Goals

- replacing the kernel with a gateway-owned policy layer
- making ACP the public product API by default
- introducing a hosted or multi-tenant control plane
- implying that public or remote exposure is safe by default
- rewriting the existing CLI surfaces before the shared substrate exists

## Current Foundation

### Kernel

The kernel remains the only place that can authoritatively decide whether a
governed action is allowed, denied, or requires approval.

The product layer may expose better status, routing, and operator workflows, but
it must not become a shadow policy engine.

### ACP

ACP is already a real internal control plane. It owns:

- backend selection
- ACP session lifecycle
- session binding and route reuse
- active turn serialization
- backend observability and doctor surfaces

That role is valid and should continue. The mistake would be to treat ACP's
backend/session cache as the same thing as the user-facing product session
model.

### Session Repository

`SessionRepository` is already the closest thing LoongClaw has to a canonical
product session plane. It persists:

- root and delegate-child session lineage
- session state and labels
- session events
- terminal outcomes
- approval requests
- approval grants
- session observations

That makes it the right source of truth for product-facing session identity,
history, approval visibility, and cross-surface continuity.

## Core Decisions

### 1. Introduce a local product control plane above the runtime, not above the kernel

LoongClaw should add a thin local product control plane that sits above the
existing runtime and orchestration layers.

Its job is to provide one shared operator and surface contract for:

- runtime snapshot and health
- session listing and observation
- turn submission and streaming
- approval visibility and decisions
- onboarding and doctor workflows

It is not a new execution authority.

The control plane must consume existing runtime and kernel-backed semantics
rather than replacing them.

### 2. `SessionRepository` is the canonical product session plane

All new product surfaces should converge on `SessionRepository` semantics for:

- session identity
- session state
- session lineage
- event history
- terminal outcomes
- approval state

This means the first HTTP and Web UI slices should not invent a second
gateway-local session store.

If a surface needs to create or continue a session, it should do so through the
same product session plane that already backs conversation continuity and
approval visibility.

### 3. `AcpSessionStore` remains ACP-local state, not the product source of truth

`AcpSessionStore` is still necessary.

It is the right place for ACP-local backend reuse and route bindings such as:

- `session_key`
- ACP backend session metadata
- typed ACP binding scope
- ACP-local last activity and backend error state

But it should stay ACP-local.

The product control plane must not treat `AcpSessionStore` as the primary
operator-facing session database.

When ACP state needs to be visible to product surfaces, the correct direction is
projection:

- ACP status and bindings project into product-readable snapshots
- product session identity remains anchored in `SessionRepository`

### 4. Surface binding must be explicit and shared

Every user-facing surface should bind through one shared contract instead of
inventing its own runtime model.

The binding questions are simple:

- does the surface create a new root session or attach to an existing one
- does the surface use a local session id, a channel-bound conversation scope,
  or an ACP route binding
- does the surface need read-only observation, turn execution, approvals, or
  support actions

CLI, HTTP, and Web UI may present those flows differently, but they should bind
through the same session and runtime substrate.

### 5. The first platform layer is local-only by default

LoongClaw's first product control plane should be localhost-only by default.

That means:

- loopback binding by default
- explicit authentication even on local HTTP surfaces
- no silent widening into LAN or public exposure
- no multi-user or hosted assumptions in the first contract

This keeps the platform layer aligned with LoongClaw's current trust model:
private, operator-owned, and local-first.

### 6. `onboard`, `doctor`, and `status` become reusable control-plane operations

The product gap is not only chat transport.

LoongClaw already has meaningful operator flows in CLI form:

- onboarding
- doctor and repair guidance
- ACP status and observability
- runtime experiment and capability review surfaces

The local product control plane should treat these as reusable operations, not
as terminal-only UX.

That is what allows a future launcher or Web UI to remain a thin product surface
instead of a second implementation.

## Control Plane Resource Model

The shared local product control plane should grow around a small set of stable
resource groups.

### Runtime

The runtime resource group should answer:

- is the local runtime healthy
- which backend and runtime modes are active
- what operator-visible warnings or repair states exist

This should aggregate existing runtime and ACP visibility instead of creating a
new runtime state machine.

### Sessions

The session resource group should expose:

- create or continue root sessions
- list sessions
- inspect session observations
- follow event streams
- read terminal outcomes

The canonical session identity comes from `SessionRepository`.

### Approvals

The approval resource group should expose:

- pending approval requests
- approval decisions
- scoped approval grants
- execution or failure outcomes tied to a session

The product layer should present approval state, but approval authority still
belongs to kernel-governed execution.

### Support

The support resource group should expose:

- onboarding readiness and handoff
- doctor diagnostics
- repair suggestions
- local migration or configuration guidance when relevant

These are operator workflows, not browser-only features.

### Turns

The turn resource group should expose:

- submit turn
- stream turn events
- fetch final turn result

This is the minimal bridge needed for `#296`, but it should attach to the shared
session plane rather than bypassing it.

## Sequencing

### Phase 1: Converge the local product substrate

Before broad surface expansion, LoongClaw should tighten the shared substrate:

- keep `#265` focused on ACP decomposition and observability hardening
- make product-facing status and session observation reuse one consistent
  session/runtime story
- keep CLI as the first client of the shared contract while the substrate is
  extracted

This is the minimum work needed to stop future gateway and Web UI effort from
forking the runtime model.

### Phase 2: Land the minimal localhost gateway on top of the shared substrate

`#296` should be implemented as a thin local HTTP layer over the product control
plane.

The first slice should stay narrow:

- health
- authenticated turn submission
- SSE turn streaming
- non-streaming turn result fetch

That slice should reuse the canonical session plane and existing execution
semantics instead of inventing HTTP-specific session ownership.

### Phase 3: Add launcher and Web UI as clients, not as a second runtime

`#403` should consume the local product control plane rather than reaching into
provider or ACP internals directly.

That means:

- same session semantics
- same onboarding and doctor semantics
- same runtime and approval behavior
- same localhost-only default boundary

The browser surface becomes a client of the platform layer, not a competing
runtime stack.

## Why not copy the OpenClaw gateway shape

OpenClaw's gateway-heavy platform is useful as a product signal:

- cross-surface continuity matters
- session routing matters
- operator-friendly local surfaces matter

But LoongClaw should not copy the whole control-center shape verbatim.

LoongClaw's architectural advantage is the kernel membrane and its explicit
governance model.

A giant gateway that owns routing, permissions, and runtime truth would weaken
that advantage and recreate policy drift one layer higher.

The correct translation is smaller:

- preserve kernel authority
- keep ACP internal and real
- add a localhost-only product control plane for shared surface semantics

## Delivery Mapping

| Issue | Role in this design |
| --- | --- |
| `#217` | umbrella for the always-on operator/runtime platform track |
| `#265` | ACP hardening prerequisite so control-plane work does not keep growing inside large hotspots |
| `#296` | first minimal localhost gateway slice on top of the shared substrate |
| `#403` | launcher and Web UI follow-on that consumes the same control plane |
| `#293` | channel and gateway sequencing context for broader protocol work |

## Summary

LoongClaw does not need a bigger kernel and it does not need a giant gateway.

It needs a boring, explicit, localhost-only product control plane that:

- keeps the kernel as authority
- keeps ACP as an internal control plane
- uses `SessionRepository` as the canonical product session plane
- lets CLI, HTTP, and Web UI converge on one shared operator/runtime contract
