# Architecture Drift Report 2026-03

## Summary
- Generated at: 2026-03-15T06:31:12Z
- Report month: `2026-03`
- Baseline report: none
- Hotspots tracked: 4
- Boundary checks tracked: 3
- SLO status: PASS

## Hotspot Metrics
| Key | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|
| spec_runtime | `crates/spec/src/spec_runtime.rs` | 3020 | 3600 | 580 | 47 | 65 | 18 | n/a | n/a | N/A | n/a |
| spec_execution | `crates/spec/src/spec_execution.rs` | 1478 | 3700 | 2222 | 23 | 80 | 57 | n/a | n/a | N/A | n/a |
| provider_mod | `crates/app/src/provider/mod.rs` | 260 | 1000 | 740 | 6 | 20 | 14 | n/a | n/a | N/A | n/a |
| memory_mod | `crates/app/src/memory/mod.rs` | 646 | 650 | 4 | 14 | 16 | 2 | n/a | n/a | N/A | n/a |

## Boundary Checks
| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | n/a | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | n/a | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |
| spec_app_dependency | PASS | n/a | spec crate remains detached from app crate at the Cargo dependency boundary |

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

<!-- arch-hotspot key=spec_runtime lines=3020 functions=47 -->
<!-- arch-hotspot key=spec_execution lines=1478 functions=23 -->
<!-- arch-hotspot key=provider_mod lines=260 functions=6 -->
<!-- arch-hotspot key=memory_mod lines=646 functions=14 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
