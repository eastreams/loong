# First-Run Assistant Journey Design

## Problem

The current `alpha-test` baseline exposes a capable runtime, but the MVP still feels more like a
developer platform than an assistant product. The visible happy path is thin:

- first-run messaging still centers on `chat` plus internal runtime primitives
- the tool surface that users can immediately recognize is weak
- product specs cover personality and memory, but not the operational user journeys that decide
  whether the product feels coherent on day one

Compared with OpenClaw, the biggest product gap is not deep architecture. It is missing visible,
legible assistant behavior and missing specs for the first-run journey.

## Goal

Ship one narrow productization slice that makes the MVP feel more like an assistant:

1. Add a one-shot `loongclaw ask --message ...` command for fast first success.
2. Add a built-in `web.fetch` tool as the first user-visible web capability.
3. Add product specs for onboarding, doctor, channel setup, WebChat expectations, and one-shot ask.
4. Refresh README and product docs so the first-run path is `onboard -> ask/chat`.

## Non-Goals

- full browser automation
- cron, webhook orchestration, or node-based workflow parity
- new channel implementations
- redesigning the conversation runtime or ACP architecture
- broad runtime refactors unrelated to the first-run product story

## Constraints

- Reuse the current `alpha-test` architecture instead of introducing a parallel product lane.
- Keep safe defaults intact. User-visible capability must still be policy-governed and auditable.
- Preserve the current conversation turn coordinator as the core execution path.
- Make docs and product specs describe actual shipped behavior, not roadmap aspirations.

## Approach Options

### Option A: Full browser automation first

Pros:

- matches the most obvious OpenClaw differentiator
- highly visible to users

Cons:

- much larger surface area: browser runtime, sandbox policy, navigation UX, tests, docs
- high schedule and regression risk for the current MVP
- likely to spill beyond one clean PR

### Option B: SSRF-safe `web.fetch` first, browser later

Pros:

- still gives users a clearly legible web capability
- fits existing tool architecture cleanly
- can be shipped with strong safety defaults and deterministic tests
- pairs naturally with a one-shot `ask` flow

Cons:

- less flashy than full browser automation
- does not yet cover interactive browsing or page actions

### Option C: Docs/specs only

Pros:

- fastest to land
- reduces UX drift

Cons:

- does not solve the visible capability gap
- leaves the MVP feeling abstract despite better documentation

## Decision

Choose Option B.

This is the highest-leverage MVP slice because it improves the first user experience without
forcing a large runtime expansion. `web.fetch` is the right first visible capability because it is
easy to explain, useful immediately, and consistent with the project's security posture.

## Design

### 1. One-shot ask

Add a new daemon subcommand:

```bash
loongclaw ask --message "Summarize this repo"
```

Behavior:

- shares the same startup/config/runtime initialization as `chat`
- executes exactly one assistant turn through `ConversationTurnCoordinator`
- prints a single `loongclaw> ...` response and exits
- respects the same session selection, provider selection, ACP options, and memory behavior as CLI
  chat

Implementation direction:

- keep `ConversationTurnCoordinator::handle_turn_with_address_and_acp_options(...)` as the single
  turn engine
- extract shared CLI bootstrap logic from `run_cli_chat(...)`
- add `run_cli_ask(...)` beside the chat CLI implementation
- keep new public API surface minimal

### 2. Built-in `web.fetch`

Add a new runtime tool named `web.fetch`.

Behavior:

- fetches a remote page over HTTP(S)
- blocks localhost, loopback, private, link-local, reserved, and unspecified IP targets by default
- rejects redirects unless every redirect target also passes validation
- caps response size and request time
- returns readable content plus metadata instead of raw unsafe HTML

Planned user value:

- agent can fetch and read documentation/articles/pages
- users can see a concrete, understandable capability in tool catalogs and docs

Safety model:

- enabled through `[tools.web]`
- enabled in the MVP build, with strict runtime limits and host validation by default
- policy includes allow/block domain controls plus private-host protection
- tests may use an explicit config override to allow local hosts without weakening production
  defaults

### 3. Product specs

Add missing product specs so UX does not drift:

- onboarding: first-run setup and first success path
- doctor: repair-oriented diagnostics expectations
- channel setup: how a user understands and configures channel surfaces
- WebChat: expectations for the future browser-facing chat surface, kept aligned with current MVP
- one-shot ask: contract for the new CLI fast path

### 4. Product-facing docs

Update README and product docs to reflect shipped behavior:

- first-run path becomes `onboard`, then `ask` or `chat`
- user-facing command table replaces stale `setup`
- visible tool surface highlights `web.fetch`
- docs explain that browser/web automation is staged, with `web.fetch` as the current MVP web
  capability

## Testing Strategy

Follow TDD for each slice:

1. write failing CLI tests for `ask`
2. implement minimal `ask` path until tests pass
3. write failing config/catalog/execution tests for `web.fetch`
4. implement minimal safe executor until tests pass
5. update docs/specs and verify command/help surfaces stay aligned

Verification target:

- targeted Rust tests for daemon CLI and app tool surfaces
- formatting
- broader package tests where the changed seams require them

## Risks And Mitigations

### Risk: `ask` forks from `chat`

Mitigation:

- extract shared bootstrap logic once
- reuse the existing single-turn coordinator path

### Risk: `web.fetch` weakens SSRF protections for tests

Mitigation:

- keep production default strict
- add an explicit local-test override in config/runtime policy
- test the validator separately from the relaxed integration path

### Risk: docs drift from shipped capability

Mitigation:

- update product specs, product sense, roadmap references, and README in the same PR

## Acceptance Criteria

- `loongclaw ask --message ...` exists and reuses the normal CLI conversation runtime
- `web.fetch` is exposed in the runtime tool catalog, provider schemas, and capability snapshot
- `web.fetch` enforces SSRF-safe defaults and runtime limits
- product specs cover onboarding, doctor, channel setup, WebChat, and one-shot ask
- README and product docs describe the real first-run path and visible capability story
