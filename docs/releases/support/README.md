# Release Support Docs Convention

This directory remains in the public repository because release-governance
scripts and checks depend on it.

It is maintainer support material for public release quality, not part of the
primary reader-facing docs navigation.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read public release history | the top-level `../vX.Y.Z*.md` notes, `../*-announcement.md`, `CHANGELOG.md`, and GitHub Releases |
| prepare or review a release document | this file and `TEMPLATE.md` |
| inspect monthly architecture drift maintenance artifacts | the `architecture-drift-YYYY-MM.md` reports in this directory |

## Read This File When

- you are preparing, reviewing, or validating a tracked release note
- you need the repository-side rules that release-governance scripts enforce
- you are deciding whether a release-support artifact is public release history
  or maintainer-only support material

## Boundary Rules

- Released `../vX.Y.Z*.md` notes are public release material.
- This file and `TEMPLATE.md` are maintainer support documents.
- Monthly architecture drift reports are repository maintenance artifacts, not
  part of the normal public release path.
- Local `.docs/` traces and debug logs are never part of the public docs path.

## Canonical Release Artifacts

Each released version in `CHANGELOG.md` (for example `## [0.1.0]` or
`## [0.1.0-alpha.1]`) must map to one tracked release process document:

- `docs/releases/v0.1.0.md` or `docs/releases/v0.1.0-alpha.1.md` (tracked, reviewed, shared)

Optional local-only support artifacts:

- `.docs/releases/v0.1.0-debug.md` or `.docs/releases/v0.1.0-alpha.1-debug.md` (ignored by git; stores intermediate retrieval/debug context)
- `.docs/traces/index.jsonl` (ignored by git; append-only trace index)

## Required Sections In Tracked Release Docs

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

## Required Consistency Rules

- `## Artifacts` must include an artifact table header (`| Asset | ...`).
- `## Detail Links` must include at least three markdown links to concrete
  evidence (workflow run, release page, changelog entry, etc.).
- `## Summary` must include trace linkage fields (`Trace ID`, `Trace path`).
- `## Process` must include an explicit `Refactor budget item:` entry.
- `Trace directory` in `## Detail Links` must exactly match the `Trace path`
  summary field.
- `Local debug log` in `## Detail Links` must exactly match
  `.docs/releases/<tag>-debug.md` for the same release tag.
- `Trace path` must stay under `.docs/traces/`, include `-post-release-`, and
  end with `-<tag>-<trace-id>` so the summary `Trace ID` and path basename
  cannot drift apart.

When local `.docs/` artifacts exist, they must exactly match the tracked release doc:

- `.docs/releases/<tag>-debug.md` must keep the same `Trace ID` and `Trace path`
- `.docs/traces/latest` must match the highest released `Trace path`
- `.docs/traces/by-tag/<tag>/latest` must match that release's `Trace path`
- `.docs/traces/index.jsonl` must include an exact success record for the release doc
- `${trace_path}/metadata.json` must mirror the same tag, trace id, trace path, command, status,
  and source release doc

## Maintainer Workflow

1. Start from `docs/releases/support/TEMPLATE.md` when preparing a release.
2. Write or review the tracked `docs/releases/vX.Y.Z*.md` release note before
   treating local debug artifacts as canonical.
3. Use `scripts/bootstrap_release_local_artifacts.sh` to regenerate local
   `.docs/` release debug and trace artifacts from tracked release docs before
   strict local doc-governance checks.
4. Use `scripts/generate_architecture_drift_report.sh` when the release needs a
   monthly architecture drift artifact.
5. Run the repository docs checks before shipping release-doc changes.

Canonical public repository links in release docs and issue templates must point to:

- `https://github.com/eastreams/loong`

This is enforced by `scripts/check-docs.sh`.

## Strictness Modes

- Local development default: release debug/trace artifacts under `.docs/` are warned but not blocking.
- CI/release default: strict mode is enabled automatically (`CI=true` / `GITHUB_ACTIONS=true`), and missing `.docs/` artifacts fail the check.
- Manual override: set `LOONGCLAW_RELEASE_DOCS_STRICT=1` for strict mode or `LOONGCLAW_RELEASE_DOCS_STRICT=0` for warn-only mode.
