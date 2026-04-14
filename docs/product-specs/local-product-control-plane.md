# Local Product Control Plane

## User Story

As a LoongClaw operator, I want every local product surface to use one shared
localhost-only control plane so that sessions, approvals, status, onboarding,
and future workflow or browser surfaces all behave like the same assistant
runtime.

## Product Scope

The local product control plane is the shared surface contract for:

- runtime health and status
- session creation, continuation, and observation
- turn submission and streaming
- approval visibility and decisions
- onboarding and doctor workflows
- future workflow and background-task observation routes that must stay rooted
  in the same session/runtime model

It is a local product substrate.

It is not a hosted control panel, a public admin API, or a second assistant
runtime.

## Current shipped slice

The current localhost control-plane slice now includes:

- authenticated runtime snapshot and event feeds
- session, approval, pairing, and ACP session observation routes
- repository-backed session observation that surfaces workflow metadata for
  visible sessions instead of only raw session fields
- repository-backed background-task observation routes for visible task-shaped
  child-session work
- authenticated turn submission
- SSE turn-event streaming for submitted turns
- non-streaming final turn-result fetch for submitted turns

Turn execution still reuses the existing ACP conversation preparation path and
the current session/runtime addressing model. The first turn-result cache stays
runtime-local; it does not introduce a second durable session authority. The
runtime-local turn registry only retains a bounded recent window of completed
turns for replay and final-result fetch.

Workflow and task productization remain split today: sessions and approvals
already flow through the control plane, while the current `loong tasks` surface
is still CLI-owned. The follow-on contract should close that gap without
inventing a separate workflow-specific identity model.

## Acceptance Criteria

- [ ] LoongClaw defines one localhost-only product control plane that future
      HTTP and Web UI surfaces consume instead of inventing separate runtime
      semantics.
- [ ] The control plane reuses the same session model across CLI and future
      browser surfaces instead of creating gateway-local session ids with
      unrelated lifecycle rules.
- [ ] Approval visibility and decisions stay consistent with the kernel-governed
      execution path rather than being reimplemented in a browser-only layer.
- [ ] `status`, `onboard`, `doctor`, and future workflow or task observation can
      be exposed as reusable local control-plane operations instead of staying
      surface-specific behavior.
- [ ] Future workflow and task routes reuse the same session lineage and
      runtime-self continuity model rather than inventing workflow-local truth.
- [ ] The first browser-facing surfaces remain localhost-only by default and do
      not imply that public exposure is supported or safe.
- [ ] The control plane remains a thin product layer above the runtime and does
      not become a second policy authority above the kernel.

## Out of Scope

- public internet exposure by default
- multi-user or hosted deployment semantics
- replacing CLI onboarding or doctor as supported operator paths
- treating ACP backend state as the canonical product session database
- creating a browser-only config or conversation model
