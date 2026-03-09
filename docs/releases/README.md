# Release Docs Convention

Each released version in `CHANGELOG.md` (for example `## [0.1.0]`) must map to a canonical release process document:

- `docs/releases/v0.1.0.md` (tracked, reviewed, shared)

Each release may also include a local-only agent debug log:

- `.docs/releases/v0.1.0-debug.md` (ignored by git; stores intermediate retrieval/debug context)
- `.docs/traces/index.jsonl` (ignored by git; append-only trace index)

Required sections in each canonical release document:

- `# Release vX.Y.Z`
- `## Summary`
- `## Process`
- `## Artifacts`
- `## Verification`
- `## Known Issues`
- `## Rollback`
- `## Detail Links`

`## Artifacts` must include an artifact table header (`| Asset | ...`).
`## Detail Links` must include at least three markdown links to concrete evidence (workflow run, release page, changelog entry, etc.).
`## Summary` must include trace linkage fields (`Trace ID`, `Trace path`).

Start from `docs/releases/TEMPLATE.md` when preparing a release.

Canonical public repository links in release docs and issue templates must point to:

- `https://github.com/loongclaw-ai/loongclaw`

This is enforced by `scripts/check-docs.sh`.
