# Automation Triggering

This document defines the first durable triggering model for Loong automation.

## Problem

Loong already had two important pieces:

- a truthful background-task surface built on detached child sessions
- inbound webhook/channel runtime paths that can already enter the shared turn runtime

What was missing was a small operator-facing trigger model that could answer
three different needs without splitting execution ownership:

- schedule work for later
- trigger work from external events such as webhooks
- leave room for future lifecycle hooks without creating a third automation
  state model

## Reference Synthesis

The reference projects split into a few stable patterns:

- `openclaw` treats cron, heartbeat, hooks, and webhooks as one product area,
  but still routes execution back into task and session surfaces
- `hermes-agent` uses a dedicated cron subsystem with durable job storage,
  a daemon tick loop, file locking, and explicit delivery paths
- `badlogic/pi-mono` emphasizes extension hooks and event-bus style triggering
  more than scheduler ownership
- `openai/codex` models hooks as structured runtime events with typed payloads
  and execution summaries, not as a second agent runtime
- `paseo` persists schedules as first-class records with cadence, target, and
  run history

The common lesson is that timing, event ingress, and hook surfaces should be
different trigger sources that converge on one execution substrate.

## Chosen Model

Loong now uses a small trigger record with:

- one durable trigger id and status
- one trigger source
- one action
- last-fire bookkeeping plus bounded run history

### Trigger Sources

- `schedule`
  - one-shot via `next_fire_at_ms`
  - recurring interval via `interval_ms`
- `cron`
  - 5-field cron expression with a materialized `next_fire_at_ms`
  - current first slice evaluates cron in UTC
- `event`
  - exact named event match such as `github.pr.opened` or
    `session.compaction.completed`
  - optional payload conditions via JSON-pointer `exists`, equality, or text-contains matching

Webhook ingress is intentionally modeled as a transport that emits named events
instead of as a separate trigger type. That keeps external webhooks and future
internal hooks on the same surface.

### Actions

The first action type is `background_task`, which queues the existing detached
delegate/task workflow instead of inventing a second worker runtime.

That choice keeps:

- session lineage
- approval truth
- timeout handling
- task inspection
- runtime diagnostics

inside the already-shipped task substrate.

## Internal Event Substrate

Loong now has an app-owned internal event publication seam plus a durable
journal:

- producers emit named internal events from app/runtime surfaces
- internal events are appended to `internal-events.jsonl`
- automation consumers can read from that journal with a persisted cursor
- the cursor now stores `segment_id`, `line_cursor`, and `byte_offset`, so
  long-running consumers can resume with incremental reads instead of
  rescanning the whole journal on every poll
- the cursor also carries a lightweight journal fingerprint so consumers can
  detect file replacement/rotation instead of blindly seeking into a different
  file that happens to be the same size or larger
- journal appends now take an OS file lock before writing, which makes the
  append path safer under concurrent emitters and gives future rotation work a
  clearer synchronization boundary
- cursor persistence now follows the same temp-write plus rename pattern as the
  automation trigger store, so serve-side cursor updates do not rely on
  truncate-in-place writes
- the journal can now be read across multiple ordered segments, which is the
  first step toward safe rotation and retention instead of treating every file
  replacement as a full replay boundary
- the active segment marker now acts as writer truth, so future appends can
  move onto a new segment without relying on “last discovered file” heuristics
- `automation serve` can now prune sealed segments that are strictly older than
  the persisted cursor segment after a successful cursor write, which gives
  Loong a first minimal retention behavior without touching the active segment
- `internal-events.state.json` is now the richer layout truth for segmented
  journals, with the legacy `internal-events.active` marker retained as a
  compatibility shadow
- operators can now inspect, rotate, and prune the automation journal through
  the automation CLI surface instead of relying on direct file manipulation
- append paths can now auto-rotate onto a fresh segment when the active
  segment exceeds the configured byte budget
  (`LOONG_INTERNAL_EVENT_SEGMENT_MAX_BYTES`), with a conservative built-in
  default

This is still a transition state rather than the final architecture. Some
surfaces still retain an immediate compatibility bridge so automation can work
without a long-running `automation serve` owner. The long-term target is
journal-first consumption, with the callback bridge demoted to compatibility or
removed entirely.

## Why Not Full Cron First

We deliberately did not start with a full cron expression engine in this slice.

Reasons:

- Loong did not already ship a cron parser in the current dependency contract
- the most important architectural decision was unifying execution, not adding
  the broadest schedule syntax
- one-shot and interval scheduling already cover the first useful operator
  workflows while keeping the surface small and reviewable

Nothing in this model blocks a future upgrade from interval scheduling to
cron-expression scheduling. That would be a source-shape expansion, not a new
automation system.

## Hooks Positioning

The current `event` trigger source is the compatibility bridge for future
hooks.

Near-term meaning:

- operators can emit named events manually
- HTTP webhook ingress can emit named events
- automation rules can subscribe to named events

Future meaning:

- runtime lifecycle surfaces can emit the same named events directly
- provider-native hook bridges can normalize external hook payloads into the
  same event names

This keeps hook adoption additive instead of forcing a later migration from
`hooks` to `events`.

## Current Internal Event Sources

The current built-in emitters cover three source families:

- app session mutation surfaces
- app work-unit repository surfaces
- daemon-side background-task operator surfaces

Successful mutations now emit named events that can be matched by `event`
triggers.

Current names:

- `background_task.queued`
- `background_task.cancelled`
- `background_task.recovered`
- `session.cancelled`
- `session.recovered`
- `session.archived`
- `work_unit.created`
- `work_unit.leased`
- `work_unit.started`
- `work_unit.heartbeat`
- `work_unit.completed`
- `work_unit.retry_pending`
- `work_unit.failed_terminal`
- `work_unit.cancelled`
- `work_unit.recovered`
- `work_unit.archived`
- `work_unit.assigned`
- `work_unit.updated`
- `work_unit.dependency_added`
- `work_unit.dependency_removed`
- `work_unit.noted`

These are not yet the full Loong hook vocabulary. They are the first stable
set of sources that proves the model:

`runtime mutation -> named event -> trigger match -> background task`

## Operational Rules

- scheduled and event-driven automation only queue background tasks; they do
  not run a shadow execution lane
- one-shot schedules complete after a successful fire
- recurring schedules reschedule from actual fire time instead of attempting
  catch-up bursts
- cron schedules also persist a materialized next-fire cursor and recompute
  from their expression after each fire
- event triggers may optionally require a JSON-pointer payload value match
- app-owned internal events now preserve `_automation.source_surface`
- internal journal consumption is a runtime substrate concern only; journal rows
  are not meant to be injected into model prompts, so this design improves
  trigger/runtime efficiency without increasing LLM token usage
  provenance so consumers can distinguish app/runtime sources from daemon-side
  bridges
- failed fires keep their trigger record and retry on a bounded later tick
- webhook ingress requires an explicit token when configured

## Follow-on Work

The intentionally reserved next steps are:

- move remaining daemon-side bridges onto the same app-owned internal event
  substrate
- runtime lifecycle emitters for hook-style events
- singleton ownership and lease-backed serving for multi-process automation
- broader retention policy and GC beyond the current minimal “older sealed
  segments only” pruning rule
- richer operator-facing journal health and repair/reporting surfaces
- policy/config hardening around automatic rotation thresholds and retention
  controls
