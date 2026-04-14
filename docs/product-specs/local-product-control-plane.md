# Local Product Control Plane

## User Story

As a LoongClaw operator, I want every local product surface to use one shared
localhost-only control plane so that sessions, approvals, status, onboarding,
and browser surfaces all behave like the same assistant runtime.

## Product Scope

The local product control plane is the shared surface contract for:

- runtime health and status
- session creation, continuation, and observation
- turn submission and streaming
- approval visibility and decisions
- onboarding and doctor workflows

It is a local product substrate.

It is not a hosted control panel, a public admin API, or a second assistant
runtime.

## Current shipped slice

The current localhost control-plane slice now includes:

- authenticated runtime snapshot and event feeds
- session, approval, pairing, and ACP session observation routes
- authenticated turn submission
- SSE turn-event streaming for submitted turns
- non-streaming final turn-result fetch for submitted turns
- a stable loopback gateway front door at `127.0.0.1:26306` by default, with
  explicit operator overrides through `--port`, `LOONGCLAW_GATEWAY_PORT`, and
  the deliberate ephemeral escape hatch `--port 0`

Turn execution still reuses the existing ACP conversation preparation path and
the current session/runtime addressing model. The first turn-result cache stays
runtime-local; it does not introduce a second durable session authority. The
runtime-local turn registry only retains a bounded recent window of completed
turns for replay and final-result fetch.

## Acceptance Criteria

- [ ] LoongClaw defines one localhost-only product control plane that future
      HTTP and Web UI surfaces consume instead of inventing separate runtime
      semantics.
- [ ] The control plane reuses the same session model across CLI and future
      browser surfaces instead of creating gateway-local session ids with
      unrelated lifecycle rules.
- [ ] Approval visibility and decisions stay consistent with the kernel-governed
      execution path rather than being reimplemented in a browser-only layer.
- [ ] `status`, `onboard`, and `doctor` can be exposed as reusable local control
      plane operations instead of staying terminal-only behavior.
- [ ] The first browser-facing surfaces remain localhost-only by default and do
      not imply that public exposure is supported or safe.
- [ ] The local product control plane exposes one stable default loopback
      address so browser shells and operator tooling do not need ad-hoc file
      reads just to discover the default gateway endpoint.
- [ ] Shared local gateway clients bootstrap against the stable default
      front door first and only fall back to persisted owner-state discovery
      when the operator explicitly chose an override or ephemeral port.
- [ ] Operator-facing status surfaces explain whether the active control-surface
      port came from the built-in default, config, env, or CLI override.
- [ ] The control plane remains a thin product layer above the runtime and does
      not become a second policy authority above the kernel.

## Out of Scope

- public internet exposure by default
- multi-user or hosted deployment semantics
- replacing CLI onboarding or doctor as supported operator paths
- treating ACP backend state as the canonical product session database
- creating a browser-only config or conversation model
