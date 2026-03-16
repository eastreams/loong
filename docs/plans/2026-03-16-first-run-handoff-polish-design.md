# First-Run Handoff Polish Design

## Goal

Make the first healthy LoongClaw experience feel more like an assistant and
less like a runtime by elevating the first runnable handoff in `onboard`,
`doctor`, and `chat` without changing any underlying provider, memory, or tool
semantics.

## Current State

- `loongclaw onboard` already ends with a branded success summary and structured
  next actions.
- `loongclaw doctor` already emits concrete next steps for healthy and repair
  flows.
- `loongclaw chat` already suggests a first prompt before dropping into the
  REPL.
- The remaining friction is mostly ordering and copy:
  - onboarding buries the first runnable action below saved config detail
  - doctor handoff text still reads like command taxonomy
  - chat still opens with session/runtime metadata before fully settling into an
    assistant-first posture

## Problem

The runtime behavior is now much closer to the intended MVP, but the first
impression still leaks operator-internal framing:

1. users see detailed setup state before the "what do I do now?" answer
2. healthy `doctor` output is actionable, but not yet phrased as a product
   handoff
3. `chat` still feels like entering a console before it feels like entering an
   assistant

That gap is small, but it is exactly the kind of surface-level friction that
causes the MVP to feel more technical than it is.

## Constraints

- Keep the change narrow and local to first-run UX.
- Do not add new runtime capabilities.
- Do not change `ask` execution semantics or add extra output after one-shot
  responses, because that would make the CLI less script-friendly and would
  drift from the one-shot ask product contract.
- Avoid introducing new generic rendering abstractions unless duplication
  becomes clearly harmful.

## Options

### Option A: Docs-only refresh

Update README and specs, but leave CLI surfaces unchanged.

Pros:

- minimal code risk
- easy to ship

Cons:

- does not improve the actual first impression inside the product
- users still encounter the same ordering and copy problems in the CLI

### Option B: Copy-only patch

Change some labels and headings, but keep current surface ordering intact.

Pros:

- low implementation cost
- can improve wording quickly

Cons:

- does not solve the more important issue that onboarding still presents
  inventory before action
- leaves chat startup structurally runtime-first

### Option C: Handoff-first polish with minimal structural changes

Keep the current runtime and renderers, but:

- move the primary onboarding handoff above the saved setup inventory
- tighten doctor handoff copy around user intent
- restructure chat startup into assistant-first followed by compact detail
  sections

Pros:

- directly improves the perceived product quality of the current MVP
- stays small and local
- aligns all three surfaces around the same user mental model

Cons:

- touches several separate CLI surfaces and tests
- requires care to avoid accidental scope creep into larger renderer refactors

## Decision

Choose Option C.

This is the smallest change that materially improves the first-run experience.
The right fix is not a new abstraction layer or new capability; it is a
handful of deliberate structural and copy adjustments across the existing
surfaces.

## Design

### 1. Promote the primary onboarding handoff

`render_onboarding_success_summary_with_width_and_style(...)` should move the
primary next action directly under the initial completion block, before the
saved provider/prompt/memory inventory.

The saved configuration details still matter, but they should become a secondary
"saved setup" section rather than the first thing a user must read.

### 2. Normalize the ask handoff label

`collect_setup_next_actions(...)` should stop labeling the primary ask action as
`ask example` and instead use a more product-shaped label such as `first
answer`.

This label is reused naturally by onboarding summaries, which keeps the visible
language aligned without introducing a new adapter layer.

### 3. Tighten doctor handoff copy

`build_doctor_next_steps_with_path_env(...)` should keep its current logic and
ordering, but update the healthy-state ask/chat prefixes to read more like next
steps than command categories.

Recommended direction:

- `Get a first answer`
- `Continue in chat`

### 4. Reframe chat startup

`render_cli_chat_startup_lines(...)` should open with:

- readiness confirmation
- a clear first thing to try
- a compact usage hint

Then it should separate operational metadata into compact secondary sections,
for example:

- `session details`
- `runtime details`

This keeps operator context available without letting it dominate the first
screen.

### 5. Keep one-shot ask unchanged

Do not add extra post-response guidance to `run_cli_ask(...)`.

The current first-run UX gap is already solved by improving how `ask` is
advertised from `onboard`, `doctor`, and `chat`. Changing one-shot ask output
itself would make the command noisier for script usage and violate the current
product spec.

## Non-Goals

- changing the behavior of `loongclaw ask`
- introducing WebChat
- reworking onboarding flow control or provider probing
- adding browser automation work in this slice
- building a shared UI rendering framework across all daemon/app surfaces

## Acceptance Criteria

- onboarding success summaries show the primary runnable handoff before the
  saved setup inventory
- the shared ask handoff label is product-shaped rather than `ask example`
- healthy doctor next steps read like user actions rather than command taxonomy
- chat startup leads with a first prompt and relegates runtime metadata to
  compact secondary sections
- docs/specs match the shipped first-run handoff behavior
