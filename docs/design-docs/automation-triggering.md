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
  - operators can preview the next bounded UTC fire times before persisting a
    trigger through `automation cron preview`
- `event`
  - exact named event match such as `github.pr.opened` or
    `session.compaction.completed`
  - optional payload conditions via JSON-pointer `exists`, equality, or text-contains matching

Webhook ingress is intentionally modeled as a transport that emits named events
instead of as a separate trigger type. That keeps external webhooks and future
internal hooks on the same surface.

### Operator And Agent Guidance

The intended decision order is:

- choose `schedule` when the real intent is one future run or a fixed
  every-N-seconds cadence
- choose `cron` when the real intent is wall-clock recurrence such as weekdays
  at 09:00 UTC
- choose `event` when a webhook, runtime mutation, or named internal event
  already exists and time is not the real source of truth

To keep the CLI usable for agents instead of only humans reading docs:

- `automation guide` is the decision entrypoint for picking a trigger type and
  finding the shortest correct command recipe
- `automation cron preview` is the review step before persisting a cron trigger
- trigger detail and preview output now carry “when to use” and “next step”
  guidance instead of only raw fields
- `automation serve` remains the live runner for durable cron delivery, webhook
  ingress, and journal-backed internal-event consumption

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
- manual journal pruning is now policy-aware instead of purely cursor-floor
  driven: operators can dry-run a prune plan, retain the latest sealed
  segments, and keep recently sealed segments above a minimum age threshold
- the same retention policy shape now applies to both manual prune and the
  automatic prune path inside `automation serve`, so operator dry-runs and
  steady-state runtime behavior no longer diverge
- `internal-events.state.json` is now the richer layout truth for segmented
  journals, with the legacy `internal-events.active` marker retained as a
  compatibility shadow
- operators can now inspect, rotate, and prune the automation journal through
  the automation CLI surface instead of relying on direct file manipulation
- append paths can now auto-rotate onto a fresh segment when the active
  segment exceeds the configured byte budget
  (`LOONG_INTERNAL_EVENT_SEGMENT_MAX_BYTES`), with a conservative built-in
  default
- automation policy defaults can now be carried through the config surface via
  `[automation]`; when operators run `automation serve` or
  `automation journal prune` against a Loong config file, whether by explicit
  `--config <path>` or the default config path, the loaded config supplies the
  default event path, poll cadence, retention window, and segment rotation
  threshold

This is still a transition state rather than the final architecture. Some
surfaces still retain an immediate compatibility bridge so automation can work
without a long-running `automation serve` owner. The long-term target is
journal-first consumption, with the callback bridge demoted to compatibility or
removed entirely.

## Automation Runner Ownership

`automation serve` now operates as a leased singleton owner, not just a
best-effort background loop.

- the active owner is published through `serve.lock`
- the latest observable runner state is published through `serve.status.json`
- cooperative shutdown requests are published through `serve.stop-request.json`
- runner state is keyed by an `owner_token`, so cleanup only removes lock or
  stop-request state that still belongs to the same owner
- the runner emits a heartbeat roughly every 5 seconds and becomes reclaimable
  after roughly 15 seconds without a heartbeat
- `automation runner inspect` now surfaces lease timeout and lease expiry so
  operators can see whether a slot is live or stale
- `automation runner inspect` also surfaces the effective automatic retention
  settings that the live `automation serve` owner is using
- `automation runner stop` is for graceful shutdown of a live owner
- `automation runner reclaim` is for explicitly reclaiming a stale owner slot;
  it marks the snapshot as stopped with a `stale_reclaimed` reason before
  clearing the stale owner file
- startup rejects a live owner, but it may reclaim a stale owner before
  acquiring the slot for the new serve process

This remains a leased singleton model. Multi-owner scheduling or standby/failover
coordination is still future work.

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
- cron expressions can be previewed without creating a trigger, so operators
  can validate UTC cadence and the next bounded fire times before persisting
  automation state
- when `automation serve` starts against a Loong config file without explicit
  retention or poll flags, it now falls back to `[automation]` config defaults
  instead of only hardcoded CLI defaults
- when `automation journal prune` runs against a Loong config file and omits
  explicit keep-last or minimum-age flags, it now uses the same `[automation]`
  defaults that the live runner uses
- explicit `automation serve` and `automation journal prune` flags still win
  over loaded `[automation]` defaults
- event triggers may optionally require a JSON-pointer payload value match
- app-owned internal events now preserve `_automation.source_surface`
- internal journal consumption is a runtime substrate concern only; journal rows
  are not meant to be injected into model prompts, so this design improves
  trigger/runtime efficiency without increasing LLM token usage
  provenance so consumers can distinguish app/runtime sources from daemon-side
  bridges
- the immediate callback bridge is only used when there is no live non-stale
  `automation serve` owner
- manual journal pruning can now be run as a dry-run policy evaluation before
  any segment is deleted
- automatic journal pruning now follows the same keep-last / minimum-age policy
  surface instead of a cursor-floor-only rule
- failed fires keep their trigger record and retry on a bounded later tick
- webhook ingress requires an explicit token when configured

## Follow-on Work

The intentionally reserved next steps are:

- move remaining daemon-side bridges onto the same app-owned internal event
  substrate
- runtime lifecycle emitters for hook-style events
- multi-owner scheduling or standby/failover behavior beyond the current leased
  singleton owner model
- richer retention policy and GC beyond the current cursor floor + keep-last +
  minimum-age policy
- richer operator-facing journal health and repair/reporting surfaces
- broader validation and operator-facing reporting around automatic rotation
  thresholds and retention controls
