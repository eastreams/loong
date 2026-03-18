# Release vX.Y.Z or vX.Y.Z-alpha.N

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
| loongclaw-vX.Y.Z[-suffix]-x86_64-unknown-linux-gnu.tar.gz | 0 | <sha256> | [link](https://github.com/loongclaw-ai/loongclaw/releases/download/vX.Y.Z[-suffix]/<asset>) |

## Verification
| Check | Result | Evidence |
|---|---|---|
| Release workflow completed | PASS/FAIL | [workflow run](https://github.com/loongclaw-ai/loongclaw/actions/runs/<id>) |
| Release is not draft | PASS/FAIL | [release page](https://github.com/loongclaw-ai/loongclaw/releases/tag/vX.Y.Z[-suffix]) |

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
- [Changelog entry](../../CHANGELOG.md)
- [Release workflow run](https://github.com/loongclaw-ai/loongclaw/actions/runs/<id>)
- [GitHub release page](https://github.com/loongclaw-ai/loongclaw/releases/tag/vX.Y.Z[-suffix])
- [Release workflow definition](../../.github/workflows/release.yml)
- Trace directory: `.docs/traces/<timestamp>-<command>-<tag>-<trace-id>`
- Local debug log: `.docs/releases/vX.Y.Z[-suffix]-debug.md`
