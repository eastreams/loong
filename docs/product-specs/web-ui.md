# Web UI

## User Story

As a prospective LoongClaw user, I want a browser-facing Web UI so that I can use, inspect, and configure the local runtime without staying in a terminal.

The Web UI should also make the basic LoongClaw path easier to approach for users who are less comfortable with CLI-first setup.

## Product Scope

The Web UI is expected to include:

- chat
- dashboard
- onboarding
- a lightweight debug console
- same-origin local product-mode serving
- an optional install path

## Acceptance Criteria

- [ ] The Web UI is treated as one coherent product surface rather than a chat-only browser shell.
- [ ] The Web UI reuses the same conversation, provider, tool, and memory semantics as CLI surfaces instead of creating a separate assistant runtime.
- [ ] The Web UI includes chat, dashboard, and onboarding as first-class parts of the same experience.
- [ ] The Web UI can be delivered in a same-origin local product mode suitable for optional installation.
- [ ] The optional install path is documented and supported without making installation mandatory for source users.
- [ ] The Web UI is positioned as an additional user-facing surface, not as a replacement for core CLI onboarding, doctor, or other foundational CLI flows.

## Out of Scope

- claiming GA-level stability before productization is complete
- treating the browser surface as a full CLI replacement
- expanding this spec into a hosted or multi-tenant web product
