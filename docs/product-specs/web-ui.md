# Web UI

## User Story

As a prospective LoongClaw user, I want a browser-facing LoongClaw product
surface so that I can use, inspect, and configure the current runtime without
staying in a terminal.

The Web UI should also make the basic LoongClaw path easier to approach for
users who are less comfortable with CLI-first setup while continuing to attach
to the same daemon-owned service/runtime core as CLI and future paired clients.

## Product Scope

The Web UI is expected to include:

- chat
- dashboard
- onboarding
- a lightweight debug console
- localhost-only by default in the current slice for security
- same-origin local product-mode serving in the current slice
- shared read models and APIs with CLI and gateway-owned operator surfaces
- an optional install path

## Architecture Direction

The current shipping boundary stays same-origin, localhost-only by default, and
implemented as a thin browser shell over the existing runtime.

That default bind policy is a security and rollout constraint for the current
slice, not the long-term architecture endpoint or a statement that future
service-mode, paired-client, or broader gateway capabilities are out of scope.

As gateway service work lands, the Web UI should become a first-class client of
the daemon-owned gateway surface and continue to reuse the same conversation,
provider, tool, memory, ACP, dashboard, and runtime-status semantics as CLI and
future paired clients.

The current gateway slice already provides a localhost-only authenticated
control surface for gateway owner status, channel inventory, runtime snapshot,
operator summary, and cooperative stop. The Web UI should consume that control
surface instead of inventing a second browser-only runtime contract.

The current daemon slice also includes a reusable localhost discovery/client
contract that validates loopback binding, loads the local bearer token, and
offers route-scoped helpers for the current gateway API. The Web UI dashboard
path should build on that contract instead of reading `status.json` and the
control token file independently.

## Acceptance Criteria

- [ ] The Web UI is treated as one coherent product surface rather than a chat-only browser shell.
- [ ] The Web UI reuses the same conversation, provider, tool, and memory semantics as CLI surfaces instead of creating a separate assistant runtime.
- [ ] The Web UI includes chat, dashboard, and onboarding as first-class parts of the same experience.
- [ ] The Web UI can be delivered in a same-origin local product mode and stays localhost-only by default in the current slice unless future policy and docs explicitly widen that boundary.
- [ ] The current localhost-only posture is documented as a safety default for
      the current slice, not as a product claim that future daemon-owned
      gateway service, pairing, or remote-capable architecture is unwanted.
- [ ] The optional install path is documented and supported without making installation mandatory for source users.
- [ ] The Web UI is positioned as an additional user-facing surface, not as a replacement for core CLI onboarding, doctor, or other foundational CLI flows.

## Out of Scope

- claiming GA-level stability before productization is complete
- treating the browser surface as a full CLI replacement
- implying that public internet exposure is safe or supported by default
- treating the default localhost-only posture as evidence that future
  service-mode or pairing work is out of scope
- treating the current localhost-only slice as the final architecture endpoint
- expanding this spec into a hosted or multi-tenant web product
