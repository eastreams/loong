# Release Local Artifact Bootstrap Design

Goal: remove the remaining local release-doc governance warnings without weakening the check or
moving more policy into CI.

## Problem

The tracked release docs already carry enough information to prove local release trace linkage:

- `Trace ID`
- `Trace path`
- the release tag itself

But `scripts/check-docs.sh` also expects the local derived artifacts under `.docs/` to exist:

- `.docs/releases/vX.Y.Z-debug.md`
- `.docs/traces/index.jsonl`
- `.docs/traces/latest`
- `.docs/traces/by-tag/vX.Y.Z/latest`

That left local runs in a permanent warn-only state even when the canonical release docs were
complete. The gap was not missing source data. The gap was missing deterministic materialization.

## Design

Add a single local bootstrap script:

- input: tracked release docs plus `CHANGELOG.md`
- output: derived `.docs/releases/*-debug.md`, trace metadata, index, and latest pointers

The script should:

1. treat the tracked release docs as the source of truth
2. rebuild local derived artifacts deterministically
3. fail fast if a release doc is malformed or its trace path escapes `.docs/traces/`
4. avoid CI/workflow edits and avoid introducing a new release orchestrator layer

The governance check should also reject internally inconsistent release docs:

- `Trace directory` must exactly match `Trace path`
- `Local debug log` must exactly match `.docs/releases/<tag>-debug.md`
- `Trace path` basename must include `-post-release-` and end with `-<tag>-<trace-id>`

When local derived artifacts are present, the governance check should also reject content drift
between the release doc and:

- `.docs/releases/<tag>-debug.md`
- `.docs/traces/latest`
- `.docs/traces/by-tag/<tag>/latest`
- `.docs/traces/index.jsonl`
- `${trace_path}/metadata.json`

## Why This Shape

This keeps the architecture narrow:

- tracked docs remain the authoritative release record
- local `.docs` stays derived and disposable
- the governance rule gets stricter in practice because developers can now reproduce the expected
  local state instead of bypassing warnings
- release docs cannot silently drift into self-contradictory trace/debug linkage
- trace identity cannot silently drift between the summary `Trace ID` field and the derived trace
  directory name
- derived `.docs` artifacts cannot silently drift away from the tracked release record while still
  looking superficially present

## Validation Plan

1. Add a shell regression that proves strict doc checks fail before bootstrapping.
2. Run the bootstrap.
3. Prove strict doc checks pass afterward on a clean fixture.
