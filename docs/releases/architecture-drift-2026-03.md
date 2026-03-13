# Architecture Drift Report 2026-03

## Summary
- Generated at: 2026-03-13T12:07:00Z
- Report month: `2026-03`
- Baseline report: none
- Hotspots tracked: 4
- Boundary checks tracked: 2
- SLO status: PASS

## Hotspot Metrics
| Key | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|
| spec_runtime | `crates/spec/src/spec_runtime.rs` | 2926 | 3600 | 674 | 47 | 65 | 18 | n/a | n/a | N/A | n/a |
| spec_execution | `crates/spec/src/spec_execution.rs` | 1467 | 3700 | 2233 | 22 | 80 | 58 | n/a | n/a | N/A | n/a |
| provider_mod | `crates/app/src/provider/mod.rs` | 234 | 1000 | 766 | 5 | 20 | 15 | n/a | n/a | N/A | n/a |
| memory_mod | `crates/app/src/memory/mod.rs` | 620 | 650 | 30 | 14 | 16 | 2 | n/a | n/a | N/A | n/a |

## Boundary Checks
| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | n/a | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | n/a | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |

## SLO Assessment
- Hotspot growth SLO (>10% month-over-month): PASS
- Boundary ownership SLO (helpers stay behind their module boundaries): PASS
- Overall architecture SLO status: PASS

## Refactor Budget Policy
- Monthly drift report command: `scripts/generate_architecture_drift_report.sh`
- Release checklist budget field lives in `docs/releases/TEMPLATE.md`.
- Rule: each release must name at least one hotspot metric paid down or explicitly state why no paydown happened.

## Detail Links
- [Architecture gate](../../scripts/check_architecture_boundaries.sh)
- [Release template](TEMPLATE.md)
- [CI workflow](../../.github/workflows/ci.yml)

<!-- arch-hotspot key=spec_runtime lines=2926 functions=47 -->
<!-- arch-hotspot key=spec_execution lines=1467 functions=22 -->
<!-- arch-hotspot key=provider_mod lines=234 functions=5 -->
<!-- arch-hotspot key=memory_mod lines=620 functions=14 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
