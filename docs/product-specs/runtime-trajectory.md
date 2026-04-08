# Runtime Trajectory

## User Story

As a LoongClaw operator, I want to export one persisted session or lineage
trajectory into a stable artifact so that I can replay runtime behavior,
inspect delegate subtrees, and feed governed evaluation or learning workflows.

## Acceptance Criteria

- [ ] LoongClaw exposes a `runtime-trajectory` command family with `export` and
      `show` subcommands.
- [ ] `runtime-trajectory export` can export one selected session without
      mutating runtime state.
- [ ] `runtime-trajectory export --lineage` can export the selected session's
      lineage-root tree, including delegate descendants that are visible from
      that root session.
- [ ] Exported artifacts include persisted turns, canonicalized turn records,
      session events, approval requests, terminal outcomes, and aggregate
      counts.
- [ ] Exported artifacts record both the requested session id and the resolved
      root session id even when only one selected session is exported.
- [ ] `runtime-trajectory show` round-trips a persisted artifact as JSON and
      renders an operator-oriented text summary when JSON output is not
      requested.
- [ ] Product docs describe `runtime-trajectory` as a read-only export layer
      for replay, evaluation, and future governed learning loops rather than an
      automatic optimizer or mutation surface.

## Out of Scope

- Automatic promotion or training triggered from trajectory export
- Background export daemons or continuous recording beyond existing persistence
- Rewriting session persistence schemas as part of the first export slice
