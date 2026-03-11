# LoongClaw Onboard Orchestrated Migration Design

Date: 2026-03-11
Status: Approved for implementation

## Goal

Deepen LoongClaw migration from a single-source importer into a safe, guided
first-run experience that can:

- detect legacy claw workspaces during onboarding
- generate per-source migration plans before writing anything
- recommend a single primary source by default
- optionally merge multiple sources in a restricted, deterministic way
- expose the same orchestration flow to agents through `claw-migration`
- preserve LoongClaw-native runtime identity and safety boundaries

The design keeps one migration engine. New work is orchestration, conflict
handling, and recovery, not a second parallel importer.

## Product Principles

### Principle 1: LoongClaw stays the runtime owner

Imported content may influence overlays and profile memory, but it must not
replace LoongClaw's native runtime base.

Stable invariants:

- `cli.prompt_pack_id = "loongclaw-core-v1"`
- runtime identity is always LoongClaw
- safety-oriented prompt and action boundaries remain LoongClaw-owned
- memory defaults remain LoongClaw-owned unless the operator explicitly changes
  them later

### Principle 2: Default to safe single-source migration

When multiple sources are detected, the default recommendation is still one
primary source. Multi-source merge is an explicitly selected mode, not the
default.

### Principle 3: Deterministic merge, not model improvisation

Agent assistance may explain or orchestrate the flow, but source scoring,
profile merge, conflict detection, and apply decisions must be deterministic in
code so the same inputs always produce the same output.

### Principle 4: Prompt lane and profile lane are different

LoongClaw must not automatically blend multiple legacy prompt/personality
instructions together. Prompt ownership stays single-source.

Allowed automatic merge scope:

- durable identity
- user preferences
- long-term profile notes
- structured AIEOS identity values

Not automatically merged:

- prompt/system behavior templates
- personality/system style
- heartbeat automation behavior
- connector/runtime secrets

### Principle 5: Apply must be recoverable

Every migration apply operation needs a backup and a machine-readable manifest.
Rollback is a first-class part of the design.

## Current Baseline

The current v0.1 implementation already provides:

- `plan_import_from_path(...)`
- `apply_import_plan(...)`
- daemon command `loongclawd import-claw`
- app-native tool `claw.import`
- spec extension wrapper `claw-migration`

What is missing is orchestration:

- discovering multiple candidate sources
- planning multiple sources together
- selecting a recommended primary source
- deterministic profile merge
- onboarding integration
- backup/manifest/rollback flow

## Target User Flows

### Flow A: First-run user with one legacy source

1. User runs `loongclawd onboard`.
2. LoongClaw detects one likely legacy claw workspace.
3. Onboarding shows a short migration summary and asks whether to import it.
4. If accepted, LoongClaw applies migration before provider/model setup
   continues.
5. User completes onboarding with inherited identity/preferences already in
   place.

### Flow B: First-run user with multiple legacy sources

1. User runs `loongclawd onboard`.
2. LoongClaw detects multiple candidate legacy workspaces.
3. It computes `plan` for each source and recommends one primary source.
4. User can:
   - accept recommended single-source import
   - inspect and manually choose one source
   - enable safe profile merge mode
5. If safe profile merge is selected, LoongClaw merges profile-lane content,
   keeps one prompt owner, and reports any unresolved conflicts.
6. Apply creates a backup, writes config, stores a manifest, and continues
   onboarding.

### Flow C: Agent-driven hot migration

1. Agent calls `claw-migration.discover` or `claw-migration.plan_many`.
2. Runtime returns structured source summaries and recommendation.
3. Agent may present a recommendation to the user or request merge mode.
4. If merge is requested, deterministic profile merge runs in code.
5. Agent calls `apply_selected`.
6. Rollback can be triggered later via `rollback_last_apply`.

## Architecture

## Layer 1: Migration Core

Add a new orchestration layer beside the existing single-source importer:

- source discovery
- per-source planning
- source scoring
- profile-lane normalization
- deterministic profile merge
- apply session manifest
- rollback metadata

The existing importer remains the atomic single-source planner/applicator.

### New core concepts

- `DiscoveredImportSource`
- `ImportSourceScore`
- `ImportOrchestrationPlan`
- `PrimaryImportSelection`
- `ProfileMergeEntry`
- `ProfileMergeConflict`
- `MergedProfilePlan`
- `ImportSessionManifest`

These types should live in `crates/app/src/migration/` so the daemon and agent
tooling can both reuse them.

## Layer 2: Onboard Orchestrator

Onboard will gain a pre-provider migration stage:

- detect candidate sources
- show compact plan summaries
- offer recommended single-source import
- optionally enable safe profile merge
- apply selected result
- continue into existing provider/model/personality/memory setup

This should be implemented as orchestration functions rather than by bloating
`run_onboard_cli(...)` with inline logic.

Recommended split:

- `discover_onboard_import_candidates(...)`
- `build_onboard_import_summary(...)`
- `resolve_onboard_import_strategy(...)`
- `apply_onboard_import_selection(...)`

## Layer 3: Agent / Spec Actions

The current `claw-migration` extension only wraps a single tool call. It should
be extended into a deterministic orchestration surface with action-based
payloads:

- `discover`
- `plan_many`
- `recommend_primary`
- `merge_profiles`
- `apply_selected`
- `rollback_last_apply`

The extension remains a thin wrapper around app migration core code rather than
embedding its own planner.

## Discovery Design

Discovery should be conservative and cheap.

### Candidate search roots

- explicit user-provided path, when present
- current working directory
- LoongClaw home parent or sibling directories
- common local workspace names if they exist nearby:
  - `nanobot`
  - `openclaw`
  - `picoclaw`
  - `zeroclaw`
  - `nanoclaw`
  - `workspace/`

### Discovery result

Each candidate returns:

- resolved path
- detected source type
- detection confidence
- found files summary
- whether portable migration content exists

Confidence should be deterministic:

- explicit operator path hint gets the highest weight
- matching branded stock/custom files adds weight
- AIEOS identity plus workspace files increases confidence
- empty or stock-only roots reduce confidence

If nothing crosses a minimum confidence threshold, onboarding proceeds without
migration prompt.

## Planning Design

`plan_many` should call the existing single-source planner for each candidate
and return:

- source id
- input path
- confidence score
- `system_prompt_addendum` present?
- `profile_note` present?
- warning count
- stock/nativeized file count
- preserved custom file count

This allows onboarding and agents to compare sources without applying.

## Merge Design

### Non-mergeable lane: prompt owner

Only one source may own prompt-lane content:

- prompt addendum
- prompt tone/behavior overlays
- personality-related prompt instructions

If multi-source merge is enabled, prompt owner is still selected from one
source, normally the recommended primary source or an explicit user choice.

### Mergeable lane: profile overlay

Profile merge operates over normalized entries:

- heading-based notes
- bullet preferences
- structured AIEOS values
- user/profile memory snippets

Normalization should produce:

- lane: `identity`, `preference`, `memory`, `value`, `bio`, `name`
- canonicalized text
- source id
- source confidence
- entry confidence
- optional semantic slot key

### Merge rules

1. Exact canonical duplicates collapse into one entry.
2. Structured entries win over free-form duplicates.
3. Same-slot conflicting entries produce a conflict record.
4. Low-risk conflicts may resolve by deterministic scoring:
   - explicit source preference
   - higher source confidence
   - higher entry confidence
   - more recent file timestamp, if available
5. Medium/high-risk conflicts must stay unresolved and require operator review.

### Merge output

`merge_profiles` returns:

- merged profile note preview
- selected prompt owner
- kept entries
- dropped duplicates
- resolved conflicts
- unresolved conflicts
- whether auto-apply is allowed

Auto-apply is allowed only when unresolved conflict count is zero.

## Recommendation Strategy

When multiple sources exist, recommendation should default to a single-source
choice and use a transparent score:

- explicit operator hint: highest
- custom non-stock content count
- profile richness
- prompt richness
- structured identity presence
- warning penalty

The recommendation output should include reasons, not just a score number.

Example:

- `openclaw@~/workspace/openclaw`: recommended because it contains custom
  prompt overlay, profile note, and no unresolved heartbeat/runtime warnings.

## Apply, Manifest, and Rollback

### Apply behavior

Before writing the target config:

1. load current target config if it exists
2. write a backup copy
3. write a manifest file describing the import session
4. write the new config

### Manifest contents

- session id
- timestamp
- operation mode
- selected primary source
- merged sources
- prompt owner source
- output path
- backup path
- warnings
- conflict summary
- content hashes or source fingerprints

### Rollback behavior

Rollback restores the backup from the last successful apply for the target
config path and records a rollback event in the manifest log.

Rollback should be available to:

- CLI
- `claw.import`
- `claw-migration`

## Non-Interactive Policy

Non-interactive mode stays more conservative than interactive mode.

Allowed:

- zero-source onboarding
- single-source recommended import
- multi-source `plan_many`

Blocked unless explicitly opted in:

- multi-source auto-merge apply
- unresolved-conflict apply

Required explicit opt-in should look like:

- merge strategy flag
- risk acknowledgement flag

## Safety Notes

- imported brand references still normalize to `LoongClaw`
- secrets are never imported
- runtime/heartbeat automation is never automatically activated
- prompt-lane multi-source mixing is disallowed by design
- merge decisions are code-driven and inspectable

## Testing Strategy

### App migration tests

- discovery returns expected candidates for mixed fixture roots
- plan_many returns deterministic ordering and scores
- profile merge deduplicates exact matches
- profile merge flags same-slot conflicts
- apply writes backup + manifest
- rollback restores previous config

### Onboard tests

- zero-source onboarding path stays unchanged
- single-source onboarding prompts for import
- multi-source onboarding recommends single source by default
- merge mode only merges profile lane
- unresolved conflicts block non-interactive apply

### Spec/runtime tests

- `claw-migration discover` returns detected candidates
- `plan_many` returns summaries for multiple roots
- `merge_profiles` returns structured conflict report
- `apply_selected` writes manifest/backup
- `rollback_last_apply` restores the previous config

## Recommended Delivery Sequence

1. Add migration orchestration core: discovery, plan-many, scoring.
2. Integrate onboarding discovery and single-source recommendation.
3. Add deterministic profile normalization and merge engine.
4. Expand `claw-migration` actions to orchestration verbs.
5. Add backup/manifest/rollback.
6. Add non-interactive safety gates and final docs.

This sequence preserves a usable product state after each stage and avoids
shipping unsafe multi-source apply behavior ahead of conflict handling.
