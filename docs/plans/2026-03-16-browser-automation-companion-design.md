# Browser Automation Companion Design

## Problem

LoongClaw now ships a truthful first-MVP browser surface through
`browser.open`, `browser.extract`, and `browser.click`. That closes the gap
between "no visible web capability" and "the assistant can safely inspect and
follow public pages".

It does not yet close the next user-perceived gap:

- non-developer users still cannot see LoongClaw complete common browser tasks
  such as login, navigation, form entry, or screenshots
- the current browser surface is intentionally limited to bounded HTML/session
  navigation and cannot become a task-execution browser without breaking its
  safety and packaging goals
- simply preinstalling an `agent-browser` skill would help power users, but it
  would not make the capability truthful, supported, or reliably available for
  normal users

Compared with OpenClaw, the next meaningful browser gap is not "missing every
browser feature they expose". It is that LoongClaw still lacks a governed,
installable, non-terminal path from "inspect a page" to "complete a page
workflow".

## Goal

Define the next browser automation phase as a LoongClaw-native product slice
that:

1. preserves the shipped lightweight browser tools as the default safe lane
2. adds a managed browser automation companion for richer page tasks
3. keeps installation friction controlled through optional packaging rather than
   forcing every user to install a heavyweight browser stack
4. exposes richer browser automation only when the companion runtime is healthy,
   enabled, and policy-allowed
5. prepares WebChat or future control surfaces to reuse the same assistant
   semantics instead of inventing a separate runtime

## Non-Goals

- replacing the shipped lightweight browser tools with a heavyweight browser
  dependency
- making Chromium/Playwright-style automation mandatory in the default install
- giving the model raw shell-level access to an unmanaged browser CLI as the
  primary browser API
- using browser automation as a shortcut to store or transmit user passwords
- shipping WebChat, dashboard controls, cron, or broader trigger automation in
  this slice

## Constraints

- LoongClaw must remain truthful about what is actually usable. Browser
  automation tools must not be advertised before the companion runtime, profile,
  and policy gates are ready.
- The base install story must continue to optimize for low friction. Heavy
  browser dependencies can only land as an optional pack or companion runtime,
  not as a new universal requirement.
- The browser companion must fit LoongClaw's existing security model:
  capabilities, approval gates, auditability, deterministic failure messages,
  and user-visible health checks.
- Browser profile isolation is mandatory. The product must not default to
  reusing a user's personal browser profile.

## Approach Options

### Option A: Preinstall only the `agent-browser` skill

Ship a bundled or preinstalled skill that teaches the agent how to call an
external `agent-browser` CLI.

Pros:

- lowest implementation cost
- gives advanced users a fast path immediately
- creates a reusable workflow template for future browser sessions

Cons:

- the runtime still depends on a separately installed browser CLI and browser
  engine
- the capability is not truthful for normal users unless onboarding/doctor also
  manage installation state
- model-facing execution would still tend to collapse into `shell.exec` instead
  of a governed browser API
- session lifecycle, profile health, approval semantics, and structured errors
  remain outside LoongClaw's main product surface

### Option B: Managed browser automation companion with an optional helper skill

Keep the current lightweight browser tools, add a first-party managed companion
runtime for richer automation, and optionally preinstall a helper skill on top
of that runtime.

Pros:

- preserves the low-friction shipped browser lane for public web inspection
- adds a believable "page task execution" lane without polluting the default
  install or tool surface
- lets `onboard`, `doctor`, install scripts, and future WebChat reuse the same
  runtime truth about readiness
- keeps LoongClaw in control of capability policy, approval, audit events, and
  tool schemas

Cons:

- requires new install/health/profile management work
- adds a second browser lane that must be explained clearly
- still needs careful tool design to avoid exposing a raw automation substrate

### Option C: Full browser runtime inside core LoongClaw

Make heavy browser automation a built-in always-available core tool family.

Pros:

- strongest parity signal against browser-heavy assistant products
- simplest mental model once fully shipped

Cons:

- directly conflicts with the current install-friction reduction goal
- pulls browser runtime, system dependency, and profile-management complexity
  into the default binary path
- increases support and CI surface before the non-terminal product shell exists

## Decision

Choose Option B.

LoongClaw should treat richer browser automation as an optional but
product-grade companion lane:

- the existing `browser.open` / `browser.extract` / `browser.click` tools remain
  the default safe browser surface
- a managed browser automation companion provides interactive page actions for
  users who opt in
- an `agent-browser`-style helper skill may be bundled as a convenience layer,
  but it is not the source of truth for runtime capability

This gives LoongClaw the best path to "assistant can complete browser tasks"
without undoing the distribution and safety gains that the MVP just made.

## Product Model

### 1. Two browser lanes, one product story

LoongClaw should explicitly present browser capabilities as two lanes:

1. **Safe Browser Lane**
   - shipped by default
   - public pages only
   - bounded HTML/session inspection
   - tools: `browser.open`, `browser.extract`, `browser.click`
2. **Automation Companion Lane**
   - optional installation
   - isolated managed browser profile
   - richer actions such as typing, waiting, screenshots, and multi-step page
     navigation
   - advertised only when the companion runtime is healthy and enabled

The user story becomes coherent:

- out of the box, LoongClaw can safely inspect the web
- with the companion enabled, LoongClaw can complete supported browser tasks

### 2. Browser automation companion packaging

The richer lane should be packaged as a first-party managed companion runtime.

Implementation direction:

- distribute the companion as a separately installable pack or release artifact
  instead of a mandatory default dependency
- keep the install path operator-friendly:
  - install script flag or optional pack selection
  - `onboard` prompt to enable enhanced browser automation
  - `doctor` checks for runtime presence, version compatibility, and browser
    profile health
- support an embedded helper skill on top of the companion for user education,
  examples, and task recipes

This keeps the base LoongClaw binary lean while still making the richer browser
lane feel first-party.

### 3. Governed adapter instead of raw shell execution

LoongClaw must not treat browser automation as "the model can call `shell.exec`
with an external browser CLI".

Instead, add a governed adapter layer that exposes stable LoongClaw-owned tool
contracts, for example:

- `browser.session.start`
- `browser.navigate`
- `browser.snapshot`
- `browser.click`
- `browser.type`
- `browser.wait`
- `browser.extract`
- `browser.screenshot`
- `browser.session.stop`

These tool names are illustrative, but the important part is the contract:

- session-scoped operations
- typed payloads and typed failures
- capability-aware authorization
- audit events on reads, writes, and terminal actions
- runtime visibility driven by actual companion readiness

The companion runtime may be implemented using an upstream browser automation
engine, but the public tool surface belongs to LoongClaw.

### 4. Action classes and approval model

The richer browser lane should distinguish between low-risk and high-risk
actions:

- **Read-oriented actions**
  - navigate to public pages
  - snapshot page structure
  - extract text or links
  - take screenshots
- **Write-oriented actions**
  - type into fields
  - click submit buttons
  - upload files
  - trigger downloads
  - confirm irreversible page actions

Policy direction:

- read-oriented actions may run under the normal browser capability lane
- write-oriented actions should require stronger policy and, when appropriate,
  explicit user approval
- approval prompts should speak in page-task language, not raw technical
  command language

### 5. Profile and login model

The product should default to an isolated LoongClaw-managed browser profile.

Rules:

- never default to the user's personal browser profile
- store companion profile state under a LoongClaw-owned runtime directory
- let users log in manually inside the isolated browser when required
- let LoongClaw reuse that isolated profile for later automated sessions
- do not make "give the agent my username and password" the default story

This keeps the product explainable and avoids turning browser automation into a
secret-handling shortcut.

### 6. Tool visibility and UX

The companion lane must integrate with the same truthfulness rules the MVP now
uses for other tools.

When the companion is unavailable, unhealthy, disabled, or policy-blocked:

- companion tools are absent from capability snapshots
- provider tool definitions omit them
- `ask` and `chat` do not hint at them as available
- `doctor` tells the user exactly what to do next

When the companion is healthy:

- `onboard` can recommend an enhanced browser automation example
- `doctor` can confirm that richer browser automation is ready
- future WebChat or Control UI work can reuse the same state instead of adding a
  shadow browser runtime

### 7. Role of a preinstalled helper skill

Bundling or preinstalling a browser helper skill is still useful, but only as a
secondary layer.

It should provide:

- natural-language browser task recipes
- user-facing examples
- safe defaults for common flows
- troubleshooting guidance

It should not be treated as:

- the only browser automation integration
- a substitute for install/doctor/runtime health plumbing
- permission to advertise an unavailable capability

## Rollout Plan

### Phase 1: Companion-aware docs and health model

- define the companion lane in product docs and roadmap
- add onboarding and doctor expectations for optional enhanced browser
  automation
- optionally bundle a helper skill for advanced users, but keep it clearly
  labeled as preview guidance

### Phase 2: Managed companion runtime and health checks

- add runtime configuration for companion enablement
- add install/onboard/doctor checks for presence, version, and profile health
- add truthful capability advertising rules for the companion lane

### Phase 3: Governed browser automation adapter

- add typed tool contracts for richer browser sessions
- map those tools to the companion runtime
- enforce read/write policy separation and approval semantics
- emit structured audit events

### Phase 4: Thin non-terminal surfaces

- let WebChat or a future Control shell reuse the same browser automation lane
- keep the shell thin and reuse `ask` / `chat` semantics instead of forking a
  separate assistant runtime

## Testing Strategy

The implementation track should require evidence at four layers:

1. **Packaging tests**
   - companion install path selection
   - version compatibility
   - failure messaging for missing runtime pieces
2. **Runtime truth tests**
   - advertised tools match actual runtime readiness
   - companion-disabled configs hide the richer browser tools
3. **Policy tests**
   - read and write actions take the correct approval path
   - session and profile isolation remain scoped
4. **Operator UX tests**
   - `onboard` offers clear opt-in copy
   - `doctor` explains missing companion prerequisites as next actions
   - first-run examples only appear when the companion is truly usable

## Risks And Mitigations

### Risk: the helper skill is mistaken for the actual browser integration

Mitigation:

- make the skill optional or clearly marked as a helper layer
- keep runtime health, tool visibility, and execution in the companion adapter
  path

### Risk: the companion drags too much weight into the base install

Mitigation:

- package it as an optional install pack
- keep the safe browser lane intact for default installs

### Risk: users trust the companion with secrets too early

Mitigation:

- use isolated profiles
- default to manual login in the managed browser
- gate write actions and irreversible page actions more strictly than reads

### Risk: browser capability copy drifts from actual runtime readiness

Mitigation:

- derive all user-facing advertising from the same runtime health and tool
  visibility path
- add tests that compare capability snapshot output against actual companion
  readiness

## Acceptance Criteria

- LoongClaw docs clearly distinguish the shipped safe browser lane from the
  planned automation companion lane
- the roadmap and product specs define browser automation companion work as the
  next browser-capability step before WebChat
- the implementation plan breaks the work into packaging, health, policy,
  runtime, and UX slices
- GitHub delivery artifacts can track this work without conflating it with the
  already shipped minimal browser surface
