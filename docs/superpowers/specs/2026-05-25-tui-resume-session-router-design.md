# TUI Resume And Session Router Design

## Summary

This design upgrades the TUI chat surface from a single-session view with ad hoc
session commands into a routed multi-session host. The first consumer is a
working `/resume` command that can hot-switch the active session inside the
current TUI process.

The user-visible goal is simple:

- `/resume` opens a searchable picker of resumable root conversations
- choosing a target hot-switches the current TUI to that session
- the switch is confirmed when transient UI state would be discarded
- the switch is blocked while a turn is pending
- `/resume latest` resumes the first eligible item
- `/resume <session-id>` allows direct recovery of any existing session, even if
  it is normally excluded from the picker

The implementation goal is broader:

- establish a real session-routing boundary inside the TUI
- avoid embedding session switching as a special case inside one command branch
- make future `/new`, `/fork`, and session-browser work reuse the same routing
  path

## Current Problem

The current chat surface exposes `/resume` in the slash command catalog, but the
runtime does not implement a real resume flow.

- `/resume` is visible and recognized as a command
- `run_surface_command(...)` does not implement a `/resume` execution branch
- unmatched commands fall back to help/detail rendering instead of session
  switching
- the current TUI runtime has no first-class "switch active session" path
- even `/new` only clears visible transcript state and does not create or bind a
  new session

At the data layer, session metadata already exists independently of transcript
turns. This means a session may exist before any user message is persisted, so
resume logic must treat "session exists" and "session is list-eligible" as
different concepts.

## Product Decisions

The following decisions were agreed during design review:

- `/resume` opens an interactive searchable picker
- the picker shows only root sessions
- the picker excludes the current session
- the picker excludes sessions that have no user messages
- picker ordering uses the latest user-message timestamp, newest first
- picker row content does not use `label`
- picker row content shows:
  - left side: the latest user-message timestamp
  - main text: the first 20 characters of the most recent user message
- if a session has fewer than 20 characters, show the actual length
- `/resume latest` means "the first item that would appear in the picker"
- `/resume <session-id>` may restore any existing session, even if it would not
  appear in the picker
- if a turn is pending, `/resume` is forbidden and should show a clear message
- if switching would discard transient UI state, the user must confirm
- the switch must be a true in-process hot switch, not a process restart

## Scope

### In Scope

- add a routed session host abstraction to the TUI chat surface
- implement `/resume`, `/resume latest`, and `/resume <session-id>`
- add a session picker mode to the existing command palette
- hot-switch the active TUI session in the same process
- atomically rebuild and replace session-scoped runtime/view state
- clear transient UI state on successful switch
- keep host-level UI state across switches
- add tests for selection, filtering, switching, blocking, and failure recovery

### Out Of Scope

- redesigning `/new`
- redesigning `/fork`
- introducing a full-screen session browser
- using `label` for resume list rendering
- adding user-managed session naming metadata
- changing the existing durable session schema unless needed for the router
- making empty sessions appear in the default `/resume` picker

## Architectural Direction

### Why A Router

The current chat surface treats the active session as an implicit property of
`CliTurnRuntime`. That is adequate for startup-time selection, but it is not a
stable base for runtime switching. A real hot-switch requires one owner of
"which session is active now" and one controlled transition path that can
rebuild state safely.

The design therefore introduces a session router layer above the current
session-bound runtime.

### Target Structure

The TUI host should be decomposed into the following major pieces:

- `SessionRouter`
  - owns the active route
  - owns transition rules
  - is the only place allowed to switch sessions
- `ActiveSessionRoute`
  - holds the current session-scoped runtime and loaded session-scoped view
    state
- `SurfaceState`
  - holds TUI host state that should survive session switches
- `TransientUiState`
  - holds short-lived interaction state that must be cleared when switching

This keeps session routing explicit and prevents future session-level commands
from duplicating unsafe state mutation logic.

## State Model

### SessionRouter

The router is the single authority for the currently active session.

It should own:

- the current `ActiveSessionRoute`
- the current transition state
- routing helpers for loading targets and replacing the route

The router should expose a single switching entry point, conceptually:

```text
switch_active_session(target_session_id, reason) -> Result<(), SwitchError>
```

All session-changing commands must go through this path.

### ActiveSessionRoute

This is the fully bound current session context.

It should contain at least:

- `CliTurnRuntime`
- session transcript state loaded for rendering
- session summary/title data
- session-scoped control-plane and picker data caches that depend on the active
  session

The key rule is that this object is replaced as a unit. The design avoids
piecemeal mutation of `runtime.session_id` and related state.

### SurfaceState

This is host-level UI state that should remain stable across session switches.

Examples:

- language
- theme
- terminal geometry-derived layout settings
- global help visibility
- non-session host preferences

This state survives switching.

### TransientUiState

This is short-lived interaction state that should be discarded on successful
switch.

Examples:

- composer draft
- queued draft
- command palette query and selected index
- inline skill popup state
- open detail overlays
- current command help card
- message selection
- transcript scroll anchor
- pending command intent

This state must not silently carry into the target session.

## Router State Machine

The router should support the following explicit states:

- `Idle`
  - normal chat interaction
- `SelectingResumeTarget`
  - `/resume` picker is open
- `ConfirmingSwitch`
  - target was chosen and destructive confirmation is required
- `Switching`
  - target route is being built
- `SwitchFailed`
  - last switch attempt failed and the previous route remains active

Important constraints:

- only the router may enter `Switching`
- a failed switch leaves the prior active route untouched
- a successful switch replaces the route atomically
- `pending_turn` blocks transitions before picker/target resolution begins

## Resume Command Contract

### `/resume`

Behavior:

- verify that no turn is currently pending
- open the command palette in resume-picker mode
- populate eligible root sessions
- allow search
- selecting a session proceeds to confirm-if-needed, then switch

### `/resume latest`

Behavior:

- verify that no turn is currently pending
- resolve the first item that would appear in the picker
- if none exists, return a clear "no resumable conversations" message
- otherwise continue through the same confirmation and switch path

### `/resume <session-id>`

Behavior:

- verify that no turn is currently pending
- resolve the target session by id
- require only that the session exists
- do not require list eligibility
- continue through the same confirmation and switch path

This intentionally separates "eligible for interactive picker" from "legal
direct target".

## Resume Candidate Query

### Picker Eligibility

A session is shown in the `/resume` picker only if all of the following are
true:

- it exists
- it is a root session
- it is not the currently active session
- it has at least one persisted user turn

### Direct Resume Eligibility

A direct target supplied via `/resume <session-id>` is valid if:

- the session exists

No additional picker filters apply.

### Sort Order

Picker order is descending by latest user-turn timestamp.

This same ordering defines `/resume latest`.

### ResumeCandidate Shape

The implementation should introduce a dedicated candidate record, conceptually:

```text
ResumeCandidate {
  session_id: String,
  last_user_turn_at: i64,
  preview_text: String,
  is_list_eligible: bool,
}
```

This keeps picker rendering, direct-lookup behavior, and future session browser
expansion from depending on loosely coupled tuple data.

## Picker Presentation

The existing command palette should be extended with a dedicated resume mode
rather than treating resume targets as ordinary slash-command docs.

Each visible row should render:

- left side: last user-turn time
- main body: first 20 characters of the latest user message

Rendering rules:

- count by character, not by byte
- if fewer than 20 characters exist, display the full message
- no `session_id` shown in the default picker
- no `label` shown in the default picker

If the picker is empty, it should render a dedicated empty-state message instead
of silently falling back to slash-command detail help.

## Switching Semantics

### What "Real Hot Switch" Means

After a successful switch:

- the current TUI process stays alive
- the active `CliTurnRuntime` is rebound to the target session
- transcript visible in the message list is replaced with the target transcript
- subsequent user messages are sent into the target session
- session-scoped views now reflect the target session

This is not a restart and not a "read-only preview". It is a true reassignment
of the active routed session.

### Atomic Replacement

Switching must be all-or-nothing:

- build the full target route first
- only after the target route is complete may it replace the old route
- if any step fails, keep the old route intact

Forbidden outcomes include:

- target title with old transcript
- target transcript with old runtime session id
- cleared transient state with no successfully installed target route

## Confirmation Policy

Switching should prompt for confirmation when it would discard transient UI
state, such as:

- non-empty composer draft
- queued draft
- active command-palette query
- visible overlay/input modal

The confirmation should be attached to the router transition path, not
implemented ad hoc inside only one command case.

## Pending Turn Blocking

If the current route has a pending turn, `/resume` must be denied immediately.

This rule applies to:

- `/resume`
- `/resume latest`
- `/resume <session-id>`

The denial should happen before:

- picker construction
- direct target lookup
- confirmation prompts

This keeps the behavior predictable and avoids entering a resume flow that must
abort late.

## Data Loading Strategy

The switch pipeline should rebuild the target route through the same
session-binding startup and history-loading code paths used during initial TUI
startup wherever possible.

Recommended sequence:

1. validate switch preconditions
2. resolve target session id
3. construct a fresh target `CliTurnRuntime`
4. load target transcript/history
5. load target session-scoped auxiliary state
6. assemble a new `ActiveSessionRoute`
7. clear `TransientUiState`
8. atomically replace the old active route
9. append a non-intrusive switch success notice if desired

This gives the UX of a hot switch while keeping implementation semantics close
to "rebuild then swap".

## Failure Handling

Failures must remain explicit and non-destructive.

Cases:

- `pending_turn` exists
  - deny immediately with a clear message
- `/resume latest` has no eligible sessions
  - show a dedicated empty-state message
- `/resume <session-id>` target does not exist
  - show a dedicated not-found message
- target runtime creation fails
  - keep old route
  - show switch-failed notice
- target transcript/session-scoped state load fails
  - keep old route
  - show switch-failed notice

No hidden fallback should convert a failed resume into slash-command help or a
half-cleared UI state.

## Recommended Implementation Order

1. Introduce router/state-boundary types
2. Refactor the TUI host to read the active route through the router
3. Add command-palette resume mode and candidate rendering
4. Add candidate-query helpers
5. Implement switch precondition checks
6. Implement route rebuild and atomic replacement
7. Wire `/resume`, `/resume latest`, and `/resume <session-id>`
8. Add focused tests

This order intentionally creates the reusable routing capability before wiring
the resume command itself.

## Testing Strategy

### Command Parsing

Add tests for:

- `/resume`
- `/resume latest`
- `/resume <session-id>`
- invalid resume argument forms

### Candidate Filtering

Add tests that verify:

- only root sessions are listed
- current session is excluded
- sessions with no user turns are excluded
- ordering follows latest user-turn timestamp
- preview text uses latest user message, truncated to 20 characters

### Switching

Add tests that verify:

- successful switch changes active session id
- transcript is replaced with target transcript
- subsequent sent messages land in target session
- transient UI state is cleared
- host-level UI state is preserved

### Blocking And Failure

Add tests that verify:

- pending turn blocks all resume entry points
- `/resume latest` reports empty state when no eligible sessions exist
- `/resume <session-id>` reports missing target when not found
- route construction/load failure leaves the old route active

## Deliberate Non-Goals For First Delivery

Even though this design introduces a reusable session router, the first delivery
should not also migrate every session command.

Specifically, first delivery should not:

- make `/new` create and bind a real new session
- migrate `/fork` to the router
- add a full-screen session browser
- infer or persist user-managed naming metadata
- expand the resume picker to child/delegate sessions

The first release should establish the routed switching foundation and use
`/resume` as the only writer of that path.

## Recommendation

Proceed with the session-router architecture, but keep the first implementation
strictly focused on `/resume`.

This preserves the user's requested "real hot switch" behavior while avoiding a
fragile one-off command patch. It also creates a clean migration path for future
session-level commands without forcing those migrations into the first delivery.
