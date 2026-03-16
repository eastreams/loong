# Product Surface Productization Design

## Problem

LoongClaw's current MVP has real runtime depth, but the first user-visible product layer still has
four gaps that are easy to feel:

- install is still presented as source-first even though the repository already builds tagged
  release binaries
- the visible web capability stops at `web.fetch`, which reads like a raw tool rather than a
  guided assistant surface
- first-run commands (`onboard`, `doctor`, `ask`, `chat`) still print too much runtime state and
  too few concrete next actions
- tool visibility is derived in multiple places, which creates future drift risk between catalog,
  prompt snapshot, provider tool schemas, and what the runtime can truly execute

Compared with OpenClaw, the most meaningful gap is not "missing every product surface they have".
It is that LoongClaw still reveals platform power before assistant value.

## Goal

Ship one LoongClaw-native productization slice that:

1. makes prebuilt installation the default happy path
2. adds one visible browser-style capability without shipping a heavyweight browser platform
3. improves first-run output so success and repair flows feel guided
4. makes tool advertising truthful across docs, runtime snapshots, and provider schemas

## Non-Goals

- full Chromium / Playwright browser automation
- cron, webhook orchestration, or native node/client parity
- adding a control UI in this slice
- redesigning the conversation runtime, ACP architecture, or memory system
- changing LoongClaw's security posture to make demos look easier

## Constraints

- This work must build on the existing `ask` + `web.fetch` first-run branch instead of restarting
  from `alpha-test`.
- Installation friction should go down, not up. Any browser solution that drags in a large new
  runtime dependency is suspect in this slice.
- The browser surface must stay bounded, auditable, and explainable within the current tool/policy
  model.
- Product docs must reflect shipped behavior, not aspirational parity.

## Approach Options

### Option A: Full browser runtime first

Build a managed browser runtime with page actions, screenshots, JS execution, and richer state.

Pros:

- strongest visible parity signal versus OpenClaw
- highly intuitive to users

Cons:

- introduces large packaging, testing, and runtime complexity at the exact moment we need to reduce
  install friction
- likely forces Chromium/Playwright or equivalent system dependencies
- expands policy scope faster than the current MVP needs

### Option B: Release-first install plus lightweight HTML browser

Keep installation focused on prebuilt binaries and add a minimal browser session layer:
`browser.open`, `browser.extract`, `browser.click`.

Pros:

- directly attacks the highest-friction first-run problem
- produces a more assistant-like capability than `web.fetch` without heavy browser packaging
- fits existing reqwest/config/policy architecture
- can reuse current SSRF, allow/block, redirect, timeout, and byte-limit controls

Cons:

- not a full interactive browser
- no JS execution, screenshots, or rich DOM automation yet

### Option C: Packaging and copy only

Focus entirely on install scripts, README, and CLI copy.

Pros:

- lowest technical risk
- clearly improves first-run friction

Cons:

- does not close the "this feels like a tool runtime, not an assistant" perception gap
- leaves `web.fetch` as the only visible web affordance

## Decision

Choose Option B.

This is the highest-leverage step that matches LoongClaw's actual situation:

- release artifacts already exist, so install friction can drop immediately
- a lightweight browser session layer adds visible assistant value without shipping a heavyweight
  runtime
- first-run output polish compounds the value of `ask`
- the existing tool catalog/runtime seams already give us a natural place to make visibility
  truthful

## Design

### 1. Release-first installation

Keep source install available, but move the product default to release-backed binaries.

Implementation direction:

- add a release-aware install path to `scripts/install.sh` and `scripts/install.ps1`
- detect the current OS/architecture and download the correct GitHub Release asset
- fall back to local build only when the user explicitly opts into source install or when release
  download is unavailable
- add checksums to release artifacts so the installer can verify downloaded binaries
- update README quick start to put prebuilt install first and source install second

This keeps LoongClaw's Rust distribution model but removes unnecessary friction for end users.

### 2. Minimal safe browser automation

Add three built-in tools:

- `browser.open`
- `browser.extract`
- `browser.click`

These tools should implement a lightweight HTML browsing session, not a full browser runtime.

Behavior:

- `browser.open`
  - fetches an HTTP(S) page under the existing web safety rules
  - stores a session record with current URL, title, a compact page summary, visible links, and a
    cookie-aware client
- `browser.extract`
  - returns structured text from the current page using limited extraction modes such as
    `page_text`, `title`, `links`, or CSS-selector-based text extraction
- `browser.click`
  - follows a previously enumerated link or an explicitly matched anchor target from the current
    page
  - reuses the same session so cookie-based navigation works across steps

Explicit non-goals for this browser surface:

- no JavaScript execution
- no arbitrary DOM mutation
- no local browser control
- no screenshot or PDF capture
- no private-network escape hatch beyond the existing config policy

Safety model:

- add a dedicated browser runtime policy, but keep it semantically aligned with `tools.web`
- use the same host validation, allow/block domain rules, redirect validation, timeout, and byte
  limits as `web.fetch`
- keep per-process browser session state bounded by session count and page size limits

### 3. First-run output polish

Improve the output contract of the existing assistant commands.

`onboard`

- keep the existing success summary structure
- elevate `ask` as the first recommended action when CLI is enabled
- include a concrete suggested prompt so the user can copy-run a first success immediately

`doctor`

- keep detailed checks available
- add a recommended next-actions section derived from the failed/warned state
- make messages action-oriented, such as "set provider credentials", "rerun with `--fix`", or
  "enable a channel only after credentials are configured"

`ask`

- print a compact productized header before the answer
- show the active session/provider context in a concise way
- make the final assistant output read like a command surface, not an internal runtime log

`chat`

- keep advanced diagnostics available, but move startup copy toward a concise assistant handoff
- preserve special commands like `/help`, but reduce default startup noise

### 4. Tool visibility truthfulness

Unify tool advertising around a single rule:

> A tool is product-visible only when it is both compiled in and enabled for the active runtime
> surface under current config/policy.

Implementation direction:

- enrich tool descriptors with explicit visibility metadata where needed
- make capability snapshots, tool registry output, and provider tool definitions all derive from
  the same runtime-visible tool view
- keep planned or policy-disabled tools out of user-facing "available tools" copy
- keep child/delegate views and root views consistent with the same descriptor-driven policy

This keeps future browser growth or external-skill growth from reintroducing drift.

## Testing Strategy

Follow TDD for each slice:

1. write failing tests for install-script platform/release selection behavior
2. write failing tests for browser catalog visibility and browser tool execution behavior
3. write failing tests for onboarding / doctor / ask / chat output changes
4. write failing tests for the new unified visibility rules
5. update docs only after the shipped behavior is passing

Verification target:

- targeted Rust tests for daemon/app behavior
- shell-script tests for installer helpers
- `cargo fmt`, targeted `cargo test`, and `cargo clippy`

## Risks And Mitigations

### Risk: browser scope quietly grows into a full browser platform

Mitigation:

- keep the initial tool set to open/extract/click only
- reject JS execution and screenshot work in this slice
- encode the limitations in docs and tool descriptions

### Risk: release-first install breaks on unsupported platforms

Mitigation:

- make platform detection explicit and testable
- keep source install as a documented fallback
- fail with concrete next actions instead of generic download errors

### Risk: tool visibility still drifts across surfaces

Mitigation:

- drive all advertising from the same runtime-visible tool view
- add tests that compare snapshot output and provider definitions against the same enabled set

### Risk: first-run output loses necessary diagnostics

Mitigation:

- keep detailed information accessible
- improve the default summary/handoff rather than deleting important status data

## Acceptance Criteria

- README quick start prefers prebuilt install and release artifacts over local source build
- install scripts can download and verify the correct release asset for supported platforms
- `browser.open`, `browser.extract`, and `browser.click` are available as built-in tools with
  strict safety defaults
- onboarding success points the user to a concrete `ask` example
- doctor output includes actionable next-step guidance
- ask/chat default output is more productized and less like raw runtime state
- tool visibility, capability snapshot, and provider tool schemas stay aligned under config gating
