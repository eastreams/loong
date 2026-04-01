# Runtime Capability

## User Story

As a LoongClaw operator, I want to derive one explicit capability candidate from
one finished runtime experiment so that I can review how a successful or failed
experiment should be crystallized into a reusable lower-layer capability.

## Acceptance Criteria

- [ ] LoongClaw exposes a `runtime-capability` command family with `propose`,
      `review`, `show`, `index`, `plan`, and `apply` subcommands.
- [ ] `runtime-capability propose` creates a persisted capability-candidate
      artifact from one finished `runtime-experiment` run.
- [ ] The candidate artifact records one explicit target type:
      `managed_skill`, `programmatic_flow`, `profile_note_addendum`, or
      `memory_stage_profile`.
- [ ] The candidate artifact records one bounded scope, normalized tags, and
      normalized required capabilities without mutating live runtime state.
- [ ] When the source run still points at recorded baseline and result snapshot
      artifacts, the candidate artifact persists the snapshot-backed runtime
      delta evidence; when those recorded snapshots are unavailable, the delta
      evidence remains explicitly empty instead of guessed.
- [ ] `runtime-capability review` records one explicit operator decision
      (`accepted` or `rejected`) plus one review summary and optional warnings.
- [ ] `runtime-capability show` round-trips the persisted artifact as JSON and
      renders the review-critical fields first in text output, including a
      compact snapshot-delta summary when one exists.
- [ ] `runtime-capability index` scans persisted candidate artifacts, groups
      matching promotion intent into deterministic capability families, and
      emits a compact evidence digest for each family.
- [ ] Capability-family evidence digests surface how many candidates carried
      snapshot-backed delta evidence plus the union of changed runtime surface
      names across that family.
- [ ] Each capability family reports readiness as `ready`, `not_ready`, or
      `blocked` from explicit evidence checks rather than opaque heuristics.
- [ ] `memory_stage_profile` families stay `not_ready` unless accepted
      candidates include snapshot-delta evidence with at least one allowlisted
      changed surface: `memory_selected`, `memory_policy`,
      `context_engine_selected`, or `context_engine_compaction`.
- [ ] `runtime-capability plan` resolves one indexed family into a dry-run
      promotion plan that describes the target lower-layer artifact, stable
      artifact id, blockers, approval checklist, rollback hints, provenance
      references, and the aggregated delta-evidence digest without mutating
      runtime state.
- [ ] `runtime-capability apply` reuses the existing `plan` contract and
      materializes one deterministic lower-layer artifact only when the chosen
      family is currently promotable.
- [ ] In v1, `runtime-capability apply` supports only the
      `memory_stage_profile` target kind and persists one governed artifact
      under the family root's `memory_stage_profiles/` delivery surface.
- [ ] Re-applying the same promotable `memory_stage_profile` family is
      idempotent when the existing materialized artifact already matches the
      deterministic expected content.
- [ ] `runtime-capability apply` fails closed for unknown family ids,
      non-promotable families, unsupported target kinds, or conflicting
      existing materialized output.
- [ ] Product docs describe `runtime-capability` as the governed review layer
      above `runtime-experiment`, with `index`/readiness and `plan` forming the
      planning ladder below explicit promotion executors or any future
      automated promotion loop.

## Out of Scope

- Automatically generating or applying managed skills
- Automatically generating or applying programmatic flows
- Automatically mutating `profile_note` or runtime config
- `runtime-capability apply` support for targets other than
  `memory_stage_profile`
- Automatic promotion, rollback, or optimizer orchestration
- Persisted capability-family state or background indexing daemons
- Persisted promotion-plan artifacts or plan caches
- Candidate queues, dashboards, or autonomous ranking systems

## Dry-Run Plan Payload

`runtime-capability plan` now carries one additional dry-run payload field:
`planned_payload`.

- `planned_payload` is emitted only when the planned family target is
  `memory_stage_profile`.
- For `managed_skill`, `programmatic_flow`, and `profile_note_addendum`,
  `planned_payload` stays `null`.
- The payload is governed review data only. It does not auto-apply anything to
  runtime, and it does not yet encode executable memory-stage settings.

The JSON shape is:

```json
{
  "planned_payload": {
    "memory_stage_profile": {
      "schema_version": 1,
      "artifact_kind": "memory_stage_profile",
      "profile": {
        "id": "memory-stage-profile-...",
        "summary": "Promote governed memory pipeline intent into a reusable profile",
        "review_scope": "Governed memory pipeline promotion intent only",
        "required_capabilities": ["memory_read"],
        "tags": ["memory", "pipeline"]
      },
      "provenance": {
        "family_id": "8f5c2d1a4b7e...",
        "accepted_candidate_ids": ["capability-candidate-..."],
        "evidence_digest": {
          "changed_surfaces": [
            "context_engine_compaction",
            "memory_policy"
          ]
        }
      }
    }
  }
}
```

For v1, `planned_payload.memory_stage_profile.profile` is derived directly from
the existing proposal and planned-artifact data already present in the plan
report:

- `profile.id` comes from `planned_artifact.artifact_id`
- `profile.summary` comes from `planned_artifact.summary`
- `profile.review_scope` comes from `planned_artifact.bounded_scope`
- `profile.required_capabilities` comes from
  `planned_artifact.required_capabilities`
- `profile.tags` comes from `planned_artifact.tags`
- `artifact_kind` matches `planned_artifact.artifact_kind`

The payload provenance is intentionally compact:

- `provenance.family_id` names the indexed capability family that was planned
- `provenance.accepted_candidate_ids` includes only accepted candidates in
  stable family order
- `provenance.evidence_digest.changed_surfaces` is a compact digest built from
  accepted-candidate snapshot-delta evidence only

That compact digest is narrower than the broader family-level plan evidence.
Rejected-only or undecided-only delta surfaces may still appear under the main
report `evidence.changed_surfaces`, but they are excluded from
`planned_payload.memory_stage_profile.provenance.evidence_digest.changed_surfaces`.

## Apply v1 Materialization

`runtime-capability apply` is the first explicit governed promotion executor.
It does not mutate live runtime configuration. Instead, it materializes one
runtime-owned `memory_stage_profile` artifact from the existing dry-run plan
contract.

For v1:

- `apply` calls the same indexed-family planner internally instead of building a
  second planning path.
- The persisted artifact content is deterministic and excludes volatile
  execution-time timestamps.
- Execution-time details such as whether the file was newly written or already
  matched are reported in the apply result, not baked into the artifact body.
- The materialized artifact uses the non-`runtime_capability` schema surface
  `memory_stage_profile`, so future capability scans ignore it safely even when
  it lives under the same root.

The persisted JSON shape is:

```json
{
  "schema": {
    "version": 1,
    "surface": "memory_stage_profile",
    "purpose": "runtime_capability_apply_output"
  },
  "artifact_kind": "memory_stage_profile",
  "artifact_id": "memory-stage-profile-...",
  "delivery_surface": "memory_stage_profiles",
  "profile": {
    "id": "memory-stage-profile-...",
    "summary": "Promote governed memory pipeline intent into a reusable profile",
    "review_scope": "Governed memory pipeline promotion intent only",
    "required_capabilities": ["memory_read"],
    "tags": ["memory", "pipeline"]
  },
  "provenance": {
    "family_id": "8f5c2d1a4b7e...",
    "accepted_candidate_ids": ["capability-candidate-..."],
    "evidence_digest": {
      "changed_surfaces": [
        "context_engine_compaction",
        "memory_policy"
      ]
    }
  }
}
```
