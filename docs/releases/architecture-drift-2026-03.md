# Architecture Drift Report 2026-03

## Summary
- Generated at: 2026-03-31T08:28:06Z
- Report month: `2026-03`
- Baseline report: none
- Hotspots tracked: 14
- Boundary checks tracked: 4
- SLO status: PASS

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3455 | 3600 | 145 | 65 | 65 | 0 | 100.0% | TIGHT |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3547 | 3700 | 153 | 43 | 80 | 37 | 95.9% | TIGHT |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 375 | 1000 | 625 | 10 | 20 | 10 | 50.0% | HEALTHY |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 356 | 650 | 294 | 14 | 16 | 2 | 87.5% | WATCH |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3383 | 3600 | 217 | 8 | 12 | 4 | 94.0% | WATCH |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2698 | 2800 | 102 | 56 | 65 | 9 | 96.4% | TIGHT |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9845 | 10500 | 655 | 88 | 90 | 2 | 97.8% | TIGHT |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9759 | 9800 | 41 | 90 | 90 | 0 | 100.0% | TIGHT |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6936 | 7300 | 364 | 146 | 160 | 14 | 95.0% | TIGHT |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1771 | 6400 | 4629 | 0 | 110 | 110 | 27.7% | HEALTHY |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 10773 | 11200 | 427 | 97 | 120 | 23 | 96.2% | TIGHT |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14256 | 15000 | 744 | 54 | 70 | 16 | 95.0% | TIGHT |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6295 | 6500 | 205 | 209 | 210 | 1 | 99.5% | TIGHT |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9519 | 9800 | 281 | 228 | 250 | 22 | 97.1% | TIGHT |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (95.9%), acpx_runtime (96.4%), channel_registry (97.8%), channel_config (100.0%), chat_runtime (95.0%), turn_coordinator (96.2%), tools_mod (95.0%), daemon_lib (99.5%), onboard_cli (97.1%)
- WATCH hotspots (>=85% and <95% of any tracked budget): memory_mod (87.5%), acp_manager (94.0%)
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

<!-- arch-hotspot key=spec_runtime lines=3455 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3547 functions=43 -->
<!-- arch-hotspot key=provider_mod lines=375 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=356 functions=14 -->
<!-- arch-hotspot key=acp_manager lines=3383 functions=8 -->
<!-- arch-hotspot key=acpx_runtime lines=2698 functions=56 -->
<!-- arch-hotspot key=channel_registry lines=9845 functions=88 -->
<!-- arch-hotspot key=channel_config lines=9759 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6936 functions=146 -->
<!-- arch-hotspot key=channel_mod lines=1771 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=10773 functions=97 -->
<!-- arch-hotspot key=tools_mod lines=14256 functions=54 -->
<!-- arch-hotspot key=daemon_lib lines=6295 functions=209 -->
<!-- arch-hotspot key=onboard_cli lines=9519 functions=228 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
