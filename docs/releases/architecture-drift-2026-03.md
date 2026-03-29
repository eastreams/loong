# Architecture Drift Report 2026-03

## Summary
- Generated at: 2026-03-29T03:10:44Z
- Report month: `2026-03`
- Baseline report: none
- Hotspots tracked: 14
- Boundary checks tracked: 4
- SLO status: PASS

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 2909 | 3600 | 691 | 59 | 65 | 6 | 90.8% | WATCH |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3545 | 3700 | 155 | 43 | 80 | 37 | 95.8% | TIGHT |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 375 | 1000 | 625 | 10 | 20 | 10 | 50.0% | HEALTHY |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 356 | 650 | 294 | 14 | 16 | 2 | 87.5% | WATCH |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3327 | 3600 | 273 | 8 | 12 | 4 | 92.4% | WATCH |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2696 | 2800 | 104 | 56 | 65 | 9 | 96.3% | TIGHT |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9846 | 10500 | 654 | 88 | 90 | 2 | 97.8% | TIGHT |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9759 | 9800 | 41 | 90 | 90 | 0 | 100.0% | TIGHT |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6821 | 7300 | 479 | 145 | 160 | 15 | 93.4% | WATCH |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 6396 | 6400 | 4 | 104 | 110 | 6 | 99.9% | TIGHT |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 9964 | 11200 | 1236 | 92 | 120 | 28 | 89.0% | WATCH |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14206 | 15000 | 794 | 54 | 70 | 16 | 94.7% | WATCH |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 5002 | 6000 | 998 | 172 | 190 | 18 | 90.5% | WATCH |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9256 | 9800 | 544 | 227 | 250 | 23 | 94.4% | WATCH |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_execution (95.8%), acpx_runtime (96.3%), channel_registry (97.8%), channel_config (100.0%), channel_mod (99.9%)
- WATCH hotspots (>=85% and <95% of any tracked budget): spec_runtime (90.8%), memory_mod (87.5%), acp_manager (92.4%), chat_runtime (93.4%), turn_coordinator (89.0%), tools_mod (94.7%), daemon_lib (90.5%), onboard_cli (94.4%)
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

<!-- arch-hotspot key=spec_runtime lines=2909 functions=59 -->
<!-- arch-hotspot key=spec_execution lines=3545 functions=43 -->
<!-- arch-hotspot key=provider_mod lines=375 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=356 functions=14 -->
<!-- arch-hotspot key=acp_manager lines=3327 functions=8 -->
<!-- arch-hotspot key=acpx_runtime lines=2696 functions=56 -->
<!-- arch-hotspot key=channel_registry lines=9846 functions=88 -->
<!-- arch-hotspot key=channel_config lines=9759 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6821 functions=145 -->
<!-- arch-hotspot key=channel_mod lines=6396 functions=104 -->
<!-- arch-hotspot key=turn_coordinator lines=9964 functions=92 -->
<!-- arch-hotspot key=tools_mod lines=14206 functions=54 -->
<!-- arch-hotspot key=daemon_lib lines=5002 functions=172 -->
<!-- arch-hotspot key=onboard_cli lines=9256 functions=227 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
