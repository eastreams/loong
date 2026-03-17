# Browser Preview First-Run Loop Design

## Goal

Close the user-facing browser preview loop so LoongClaw can truthfully guide a
normal operator from "optional browser preview exists" to "I enabled it, I know
what is missing, and I have a concrete first task to try" without expanding
into a heavyweight browser runtime project.

## Current State

- LoongClaw already has a safe built-in browser lane through `browser.open`,
  `browser.extract`, and `browser.click`.
- LoongClaw also ships an optional `browser-companion-preview` bundled skill
  plus `loongclaw skills enable-browser-preview`.
- `onboard` and `doctor` already reuse `crates/daemon/src/next_actions.rs` to
  surface first-run next actions.
- The current browser preview guidance is still too operator-internal:
  - ready state hands users a prompt built around `external_skills.invoke`
    instead of a user task
  - missing runtime guidance falls back to `agent-browser --help`
  - `skills enable-browser-preview` text output is mostly key/value status
  - users are not handed 2 to 3 concrete recipes after enabling the preview
- PR #197 is already using the same first-run surfaces for build metadata and
  compact headers, so this slice must avoid overlapping that work.

## Problem

The browser preview already exists structurally, but not yet as a polished MVP
experience:

1. enabling it is one command, but understanding what to do next still requires
   reading README or the bundled skill
2. doctor can tell users the preview is missing `agent-browser`, but it does
   not yet tell them a concrete install action
3. the first runnable browser-preview example still reads like internal runtime
   mechanics instead of a user-visible task
4. first-run surfaces therefore still feel more like tooling than assistant
   value

## Constraints

- Do not turn this slice into a new managed browser runtime project.
- Do not overlap the open `#195` / PR `#197` header and build-metadata work.
- Keep the source of truth shared across `onboard`, `doctor`, and
  `skills enable-browser-preview`, instead of hand-writing separate copy in each
  command.
- Stay truthful about the current preview architecture: this is still the
  `agent-browser` companion path on top of `shell.exec`, not a new native core
  browser API.

## Options

### Option A: Docs and copy only

Update README and specs, but leave CLI behavior mostly unchanged.

Pros:

- very low implementation cost
- no behavior risk

Cons:

- the real first-run path remains broken inside the product
- users still have to infer install steps and recipes themselves

### Option B: Shared browser-preview guidance layer across CLI surfaces

Keep the current runtime shape, but add one shared guidance model that powers:

- `skills enable-browser-preview` text output
- `doctor` next steps
- ready-state browser preview first-task prompts
- product docs/spec examples

Pros:

- directly improves user-visible value without new runtime architecture
- keeps `onboard`, `doctor`, and skills CLI in sync through shared logic
- fits the current MVP need: enable, diagnose, try

Cons:

- still depends on the external `agent-browser` runtime
- requires some careful wording so preview truthfulness is preserved

### Option C: Auto-install or fully manage the browser runtime now

Make LoongClaw responsible for installing and maintaining `agent-browser`.

Pros:

- strongest product feel
- least operator setup after the fact

Cons:

- expands scope into external runtime lifecycle management
- increases platform, packaging, and support complexity immediately
- no longer a narrow MVP polish slice

## Decision

Choose Option B.

This slice should productize the existing preview path rather than replace it:

- `enable-browser-preview` remains the one-command activation path
- doctor should name the missing runtime and give the operator an exact install
  plus verify sequence
- ready browser preview states should hand users task-shaped recipes, not
  internal implementation jargon
- the same recipe and install guidance should be reusable across CLI surfaces

## Design

### 1. Shared browser preview install guidance

Add a small shared set of browser preview install and verification steps in
`crates/daemon/src/browser_preview.rs`.

This module should provide:

- one recommended install command for `agent-browser`
- one verification command
- optional supporting copy for README/spec text

The install command should be concrete and cross-platform enough for CLI copy.
The current best fit is the upstream npm path plus the runtime's own install
step, followed by `agent-browser --help`.

### 2. Shared browser preview recipes

Add 2 to 3 browser-preview first-task recipes as shared data, not ad-hoc copy.

Recommended recipes:

1. open a page and summarize what is visible
2. extract the main page text and key points
3. click a discovered link, wait for navigation, and summarize the result

Each recipe should compile down to a `loongclaw ask --config ... --message "..."`
command so users can run it immediately.

### 3. Upgrade ready-state guidance

Replace the current ready-state browser preview prompt with the first shared
recipe. The message should still be truthful about using the preview skill, but
it should read like a user task rather than "load `external_skills.invoke`".

### 4. Improve missing-runtime guidance

When the preview is enabled but `agent-browser` is missing, the CLI should show:

- install command
- verify command
- the normal `doctor` follow-up path

This should replace the current bare `agent-browser --help` action.

### 5. Productize `skills enable-browser-preview`

Keep JSON payload stability, but make text mode feel like a product command:

- show whether config was updated
- show whether the runtime is already available
- show the next steps
- if the runtime is available, show 2 to 3 recipe commands immediately
- if the runtime is missing, show install plus verify, then the same recipes as
  "after install, try"

### 6. Keep onboarding and doctor aligned

`onboard` and `doctor` should continue to consume `next_actions`, but the
browser-preview actions they see should now be better:

- enable action stays the same
- unblock action stays the same
- install-runtime action becomes a real install step
- ready action becomes a concrete first task

This keeps the surfaces aligned without duplicating UX logic.

## Non-Goals

- auto-downloading or auto-updating the `agent-browser` runtime
- replacing the built-in browser tools
- adding new browser automation core APIs
- bundling Chromium or Playwright into the main LoongClaw install
- redesigning the full `ask` or `chat` rendering stack outside this browser
  preview touchpoint

## Acceptance Criteria

- `skills enable-browser-preview` text output gives actionable next steps and
  browser task recipes.
- `doctor` next steps give a concrete `agent-browser` install action instead of
  only `agent-browser --help`.
- The browser preview ready-state action handed to `onboard` / `doctor` is a
  task-shaped recipe rather than an implementation-shaped prompt.
- Relevant product specs and README/browser preview docs match the shipped
  behavior.
