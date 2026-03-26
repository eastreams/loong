# Architecture Drift Report 2026-03

## Summary
- Generated at: 2026-03-26T12:25:05Z
- Report month: `2026-03`
- Baseline report: none
- Hotspots tracked: 14
- Boundary checks tracked: 4
- SLO status: PASS

## Hotspot Metrics
| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3289 | 3600 | 311 | 48 | 65 | 17 | 91.4% | WATCH | n/a | n/a | N/A | n/a |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 2057 | 3700 | 1643 | 29 | 80 | 51 | 55.6% | HEALTHY | n/a | n/a | N/A | n/a |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 375 | 1000 | 625 | 10 | 20 | 10 | 50.0% | HEALTHY | n/a | n/a | N/A | n/a |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 356 | 650 | 294 | 14 | 16 | 2 | 87.5% | WATCH | n/a | n/a | N/A | n/a |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3327 | 3600 | 273 | 8 | 12 | 4 | 92.4% | WATCH | n/a | n/a | N/A | n/a |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2575 | 2800 | 225 | 55 | 65 | 10 | 92.0% | WATCH | n/a | n/a | N/A | n/a |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 10104 | 10500 | 396 | 86 | 90 | 4 | 96.2% | TIGHT | n/a | n/a | N/A | n/a |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9364 | 9800 | 436 | 90 | 90 | 0 | 100.0% | TIGHT | n/a | n/a | N/A | n/a |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6818 | 7300 | 482 | 145 | 160 | 15 | 93.4% | WATCH | n/a | n/a | N/A | n/a |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 6176 | 6400 | 224 | 100 | 110 | 10 | 96.5% | TIGHT | n/a | n/a | N/A | n/a |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 9963 | 11200 | 1237 | 92 | 120 | 28 | 89.0% | WATCH | n/a | n/a | N/A | n/a |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14111 | 15000 | 889 | 54 | 70 | 16 | 94.1% | WATCH | n/a | n/a | N/A | n/a |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 5703 | 6000 | 297 | 176 | 190 | 14 | 95.0% | TIGHT | n/a | n/a | N/A | n/a |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9232 | 9800 | 568 | 227 | 250 | 23 | 94.2% | WATCH | n/a | n/a | N/A | n/a |

## Prioritization Signals
- TIGHT hotspots (>=95% of any tracked budget): channel_registry (96.2%), channel_config (100.0%), channel_mod (96.5%), daemon_lib (95.0%)
- WATCH hotspots (>=85% and <95% of any tracked budget): spec_runtime (91.4%), memory_mod (87.5%), acp_manager (92.4%), acpx_runtime (92.0%), chat_runtime (93.4%), turn_coordinator (89.0%), tools_mod (94.1%), onboard_cli (94.2%)
- Mixed-class hotspots (size plus operational density): chat_runtime, channel_mod, turn_coordinator

## Boundary Checks
| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | n/a | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | n/a | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |
| conversation_provider_optional_binding_roundtrip | PASS | n/a | conversation/runtime.rs translates explicit conversation bindings into provider bindings without optional-kernel roundtrips |
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

<!-- arch-hotspot key=spec_runtime lines=3289 functions=48 -->
<!-- arch-hotspot key=spec_execution lines=2057 functions=29 -->
<!-- arch-hotspot key=provider_mod lines=375 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=356 functions=14 -->
<!-- arch-hotspot key=acp_manager lines=3327 functions=8 -->
<!-- arch-hotspot key=acpx_runtime lines=2575 functions=55 -->
<!-- arch-hotspot key=channel_registry lines=10104 functions=86 -->
<!-- arch-hotspot key=channel_config lines=9364 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6818 functions=145 -->
<!-- arch-hotspot key=channel_mod lines=6176 functions=100 -->
<!-- arch-hotspot key=turn_coordinator lines=9963 functions=92 -->
<!-- arch-hotspot key=tools_mod lines=14111 functions=54 -->
<!-- arch-hotspot key=daemon_lib lines=5703 functions=176 -->
<!-- arch-hotspot key=onboard_cli lines=9232 functions=227 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
