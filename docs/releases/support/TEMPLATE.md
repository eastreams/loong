# Release vX.Y.Z or vX.Y.Z-alpha.N

This template remains in the public repository because release-governance automation and release
document checks depend on it. It supports public release artifacts but is not intended to behave
like a primary reader-facing product doc.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read public release history | the top-level `../vX.Y.Z*.md` or `../*-announcement.md` files |
| prepare a new release document | this template |
| understand the release-support file boundary | [`README.md`](README.md) |

## Read This File When

- you are preparing or reviewing a tracked release note
- you need the repository-side shape that release-doc checks enforce
- you are validating whether a release document still matches the current
  release-governance contract

## Summary
- Generated at:
- Release status:
- Target commitish:
- Artifact count:
- Trace ID:
- Trace path:

## Highlights
- README-level release summary bullet.

## Process
- Date:
- Owner:
- Scope summary:
- Gates run:
- Refactor budget item:

## Artifacts
| Asset | Size (bytes) | SHA256 | Download |
|---|---:|---|---|
| loong-vX.Y.Z[-suffix]-x86_64-unknown-linux-gnu.tar.gz | 0 | <sha256> | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/<asset>) |
| loong.bash | 0 | n/a | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/loong.bash) |
| _loong | 0 | n/a | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/_loong) |
| loong.fish | 0 | n/a | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/loong.fish) |
| loong.ps1 | 0 | n/a | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/loong.ps1) |
| loong.elv | 0 | n/a | [link](https://github.com/eastreams/loong/releases/download/vX.Y.Z[-suffix]/loong.elv) |

## Verification
| Check | Result | Evidence |
|---|---|---|
| Release workflow completed | PASS/FAIL | [workflow run](https://github.com/eastreams/loong/actions/runs/<id>) |
| Release is not draft | PASS/FAIL | [release page](https://github.com/eastreams/loong/releases/tag/vX.Y.Z[-suffix]) |

## Refactor Budget
- Hotspot metric paid down:
- Evidence:
- If no paydown shipped, rationale:

## Known Issues
- None / describe issue and mitigation.

## Rollback
- Mark release draft and remove broken assets.
- Publish superseding patch release and link it here.

## Detail Links
- [Changelog entry](../../../CHANGELOG.md)
- [Release workflow run](https://github.com/eastreams/loong/actions/runs/<id>)
- [GitHub release page](https://github.com/eastreams/loong/releases/tag/vX.Y.Z[-suffix])
- [Release workflow definition](../../../.github/workflows/release.yml)
- Trace directory: `.docs/traces/<timestamp>-<command>-<tag>-<trace-id>`
- Local debug log: `.docs/releases/vX.Y.Z[-suffix]-debug.md`

## Do Not Use This Template For

- public release-history reading that should use top-level `../vX.Y.Z*.md` files
  or GitHub Releases
- incident notes, private planning bundles, or scratch release checklists that
  do not belong in the tracked OSS release-doc path
- changing the public product docs navigation; this template is release-support
  material only
