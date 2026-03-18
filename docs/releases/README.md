# Release Docs Convention

Each released version in `CHANGELOG.md` (for example `## [0.1.0]` or `## [0.1.0-alpha.1]`) must map to a canonical release process document:

- `docs/releases/v0.1.0.md` or `docs/releases/v0.1.0-alpha.1.md` (tracked, reviewed, shared)

Architecture governance may also emit monthly drift reports under the same directory:

- `docs/releases/architecture-drift-2026-03.md` (tracked monthly architecture SLO snapshot)

Each release may also include a local-only agent debug log:

- `.docs/releases/v0.1.0-debug.md` or `.docs/releases/v0.1.0-alpha.1-debug.md` (ignored by git; stores intermediate retrieval/debug context)
- `.docs/traces/index.jsonl` (ignored by git; append-only trace index)

Required sections in each canonical release document:

- `# Release vX.Y.Z` or `# Release vX.Y.Z-alpha.N`
- `## Summary`
- `## Highlights`
- `## Process`
- `## Artifacts`
- `## Verification`
- `## Refactor Budget`
- `## Known Issues`
- `## Rollback`
- `## Detail Links`

`## Artifacts` must include an artifact table header (`| Asset | ...`).
`## Detail Links` must include at least three markdown links to concrete evidence (workflow run, release page, changelog entry, etc.).
`## Summary` must include trace linkage fields (`Trace ID`, `Trace path`).
`## Process` must include an explicit `Refactor budget item:` entry.
`Trace directory` in `## Detail Links` must exactly match the `Trace path` summary field.
`Local debug log` in `## Detail Links` must exactly match `.docs/releases/<tag>-debug.md` for the
same release tag.
`Trace path` must stay under `.docs/traces/`, include `-post-release-`, and end with
`-<tag>-<trace-id>` so the summary `Trace ID` and path basename cannot drift apart.
When local `.docs/` artifacts exist, they must exactly match the tracked release doc:

- `.docs/releases/<tag>-debug.md` must keep the same `Trace ID` and `Trace path`
- `.docs/traces/latest` must match the highest released `Trace path`
- `.docs/traces/by-tag/<tag>/latest` must match that release's `Trace path`
- `.docs/traces/index.jsonl` must include an exact success record for the release doc
- `${trace_path}/metadata.json` must mirror the same tag, trace id, trace path, command, status,
  and source release doc

Start from `docs/releases/TEMPLATE.md` when preparing a release.
Use `scripts/generate_architecture_drift_report.sh` to produce a monthly architecture drift artifact.
Use `scripts/bootstrap_release_local_artifacts.sh` to regenerate local `.docs/` release debug and
trace artifacts from the tracked release docs before running strict local doc-governance checks.

Canonical public repository links in release docs and issue templates must point to:

- `https://github.com/loongclaw-ai/loongclaw`

This is enforced by `scripts/check-docs.sh`.

Release artifact strictness modes:

- Local development default: release debug/trace artifacts under `.docs/` are warned but not blocking.
- CI/release default: strict mode is enabled automatically (`CI=true` / `GITHUB_ACTIONS=true`), and missing `.docs/` artifacts fail the check.
- Manual override: set `LOONGCLAW_RELEASE_DOCS_STRICT=1` for strict mode or `LOONGCLAW_RELEASE_DOCS_STRICT=0` for warn-only mode.
