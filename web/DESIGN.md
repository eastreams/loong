# LoongClaw Web Console Design

Status: proposed  
Scope: `alpha-test` lineage, single delivery PR  
Last updated: 2026-03-17

## 1. Goal

Build a Web Console for LoongClaw with two primary surfaces:

- Web Chat
- Web Dashboard

The Web Console must reuse the existing LoongClaw conversation, provider, tool,
memory, and audit semantics. It is a new client surface, not a new assistant
runtime.

## 2. Product Positioning

The Web Console is an optional frontend module.

- Base install remains CLI-first.
- Web assets are not bundled into the default install path.
- `onboard` may offer an opt-in Web Console install choice with size disclosure.
- The first priority is local deployment.
- Hosted frontend mode is a later distribution option, not an MVP requirement.

## 3. Core Decision

Adopt a local-first frontend/backend split.

- The local `loongclaw` process remains the system of record and execution runtime.
- The Web Console is a separate frontend module under `web/`.
- The backend exposes a local HTTP API control plane.
- The frontend consumes that API.

This keeps Web decoupled enough to evolve independently while preserving one
runtime path for CLI, Web, and future channels.

## 4. Non-Goals For MVP

- No separate cloud-hosted agent runtime
- No multi-user server mode
- No default public-network exposure
- No remote sync product promise
- No forced hosted frontend path
- No requirement to install Web assets during base install

## 5. Repository Layout

Recommended structure:

```text
crates/
  app/
  daemon/
web/
  src/
  public/
  package.json
  DESIGN.md
scripts/
  web/
docs/
  references/
```

Rationale:

- keep frontend and backend in one repo during rapid protocol iteration
- avoid early multi-repo coordination cost
- preserve the option to split later if release cadence diverges

## 6. Runtime Architecture

### 6.1 Backend

Backend responsibilities live in `crates/daemon` and `crates/app`.

`crates/daemon`:

- start local HTTP server
- expose Web API routes
- perform HTTP auth and origin checks
- optionally serve installed local static assets
- report install/runtime status

`crates/app`:

- keep the actual conversation runtime
- expose a reusable session/turn service for Web and CLI
- continue to own provider, tool, memory, ACP, and audit behavior

### 6.2 Frontend

The frontend under `web/` is a standalone client.

Responsibilities:

- render chat UI
- render dashboard UI
- manage local connection state
- call backend APIs
- handle auth token input/storage for local use

The frontend must not reimplement runtime logic.

## 7. Conversation Model

Web must map onto the existing conversation address model.

Recommended mapping:

- `channel_id = webchat`
- `conversation_id = <browser_session_id>`
- `thread_id = <tab_or_subthread_id>` when needed later

Ingress metadata may include browser-specific context, but it must remain routing
and UX context only, not authorization.

## 8. Primary Surfaces

### 8.1 Web Chat

MVP capabilities:

- create or resume sessions
- send one turn
- read recent history
- show response status
- show current provider/runtime summary

Deferred:

- advanced trace panes
- collaborative session views
- complex attachment workflows

### 8.2 Web Dashboard

MVP capabilities:

- runtime summary
- active provider and provider availability
- memory status
- tool availability/status summary
- config digest
- doctor/runtime warnings
- Web Console install mode and asset status

Deferred:

- full admin control plane
- multi-instance fleet views
- remote device orchestration

## 9. API Shape

Initial API surface:

- `GET /healthz`
- `GET /api/meta`
- `GET /api/chat/sessions`
- `POST /api/chat/sessions`
- `GET /api/chat/sessions/:id/history`
- `POST /api/chat/sessions/:id/turn`
- `GET /api/dashboard/summary`
- `GET /api/dashboard/providers`
- `GET /api/dashboard/tools`
- `GET /api/dashboard/runtime`
- `GET /api/dashboard/config`

Optional later additions:

- `GET /api/chat/sessions/:id/stream/:turn_id` via SSE
- richer diagnostics endpoints
- install/update endpoints for local Web assets

## 10. Install And Distribution Modes

The same protocol should support multiple delivery modes.

### 10.1 Base Install

- install CLI/runtime only
- no Web assets by default

### 10.2 Local Web Install

- user opts in during `onboard` or later command flow
- Web assets are downloaded or unpacked into a local directory
- local daemon serves static assets or the user opens them locally

Suggested config:

```toml
[webchat]
enabled = true
bind = "127.0.0.1:4317"
install_mode = "local_assets"
static_dir = "~/.loongclaw/web/current"
auth_mode = "local_token"
allowed_origins = ["http://127.0.0.1:4317"]
```

### 10.3 Hosted Frontend Mode

Later-only mode:

- frontend may be deployed remotely by LoongClaw or by users
- frontend still connects to the user's own local runtime
- requires stricter auth, origin, connection UX, and operator messaging

This mode is explicitly not the MVP priority.

## 11. Security Model

MVP defaults:

- bind to loopback by default
- require explicit local token for API access
- deny broad origin access by default
- do not expose to public network automatically

Later hosted mode concerns:

- cross-origin trust model
- local endpoint exposure education
- token lifecycle and revocation
- safe device pairing UX
- clear explanation that runtime remains local

## 12. Commands

Suggested command surface:

- `loongclaw webchat serve`
- `loongclaw webchat status`
- `loongclaw webchat install`
- `loongclaw webchat remove`

`onboard` may offer:

- `CLI only`
- `CLI + Local Web Console`

The install option should disclose approximate Web asset size.

## 13. Implementation Plan

### Phase 1: Design And Contracts

- document architecture and install modes
- define API response shapes
- define auth and local-only defaults

### Phase 2: Backend Reuse Layer

- extract reusable chat session initialization from CLI-only code
- expose a Web-safe conversation service in `crates/app`

### Phase 3: Local HTTP Control Plane

- add `webchat serve`
- implement core chat and dashboard endpoints
- implement local token auth

### Phase 4: Frontend MVP

- build chat page
- build dashboard page
- add shared API client and connection state

### Phase 5: Optional Install Flow

- build Web assets separately from core CLI
- add install/remove/status commands
- wire `onboard` selection flow

### Phase 6: Hosted Mode Preparation

- keep protocol stable
- add origin and endpoint configuration hooks
- defer hosted rollout until local mode is proven

## 14. Open Questions

- Should local static assets be served by `loongclaw webchat serve`, or only downloaded for a
  separate frontend dev/prod server model?
- Should chat responses be request/response only for MVP, or should SSE land in the first PR?
- How much dashboard control should be read-only versus action-oriented in the first release?
- How should Web asset version compatibility be enforced against the local daemon version?

## 15. Acceptance For First Delivery

The first large PR is successful when:

- Web Chat and Web Dashboard both exist
- Web uses the existing LoongClaw runtime semantics
- base install remains CLI-first
- Web assets are optional
- local deployment works without hosted infrastructure
- hosted mode remains only an architectural extension point
