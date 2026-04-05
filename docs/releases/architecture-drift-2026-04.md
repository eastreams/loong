# Architecture Drift Report 2026-04

## Summary
- Generated at: 2026-04-05T13:19:24Z
- Report month: `2026-04`
- Baseline report: docs/releases/architecture-drift-2026-03.md
- Hotspots tracked: 14
- Boundary checks tracked: 5
- SLO status: PASS

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3528 | 3600 | 72 | 65 | 65 | 0 | 100.0% | TIGHT | 3455 | 2.1% | PASS | 65 |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3573 | 3700 | 127 | 48 | 80 | 32 | 96.6% | TIGHT | 3547 | 0.7% | PASS | 43 |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 376 | 1000 | 624 | 10 | 20 | 10 | 50.0% | HEALTHY | 375 | 0.3% | PASS | 10 |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 355 | 650 | 295 | 14 | 16 | 2 | 87.5% | WATCH | 356 | -0.3% | PASS | 14 |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3391 | 3600 | 209 | 8 | 12 | 4 | 94.2% | WATCH | 3383 | 0.2% | PASS | 8 |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2946 | 2800 | -146 | 58 | 65 | 7 | 105.2% | BREACH | 2698 | 9.2% | PASS | 56 |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 10222 | 10500 | 278 | 72 | 90 | 18 | 97.4% | TIGHT | 9922 | 3.0% | PASS | 88 |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9716 | 9800 | 84 | 90 | 90 | 0 | 100.0% | TIGHT | 9796 | -0.8% | PASS | 90 |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6976 | 7300 | 324 | 147 | 160 | 13 | 95.6% | TIGHT | 6936 | 0.6% | PASS | 146 |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1785 | 6400 | 4615 | 0 | 110 | 110 | 27.9% | HEALTHY | 1779 | 0.3% | PASS | 0 |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 10833 | 11200 | 367 | 97 | 120 | 23 | 96.7% | TIGHT | 10831 | 0.0% | PASS | 98 |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14810 | 15000 | 190 | 53 | 70 | 17 | 98.7% | TIGHT | 14472 | 2.3% | PASS | 54 |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6806 | 6500 | -306 | 224 | 210 | -14 | 106.7% | BREACH | 6324 | 7.6% | PASS | 210 |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9723 | 9800 | 77 | 235 | 250 | 15 | 99.2% | TIGHT | 9519 | 2.1% | PASS | 228 |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): acpx_runtime (105.2%), daemon_lib (106.7%)
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), channel_registry (97.4%), channel_config (100.0%), chat_runtime (95.6%), turn_coordinator (96.7%), tools_mod (98.7%), onboard_cli (99.2%)
- WATCH hotspots (>=85% and <95% of any tracked budget): memory_mod (87.5%), acp_manager (94.2%)
- Mixed-class hotspots (size plus operational density): chat_runtime, channel_mod, turn_coordinator

## Boundary Checks

| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | PASS | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | PASS | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |
| conversation_provider_optional_binding_roundtrip | PASS | PASS | conversation/runtime.rs translates explicit conversation bindings into provider bindings without optional-kernel roundtrips |
| conversation_app_dispatcher_optional_kernel_context | PASS | n/a | conversation app-tool dispatcher approval hooks stay binding-based without optional kernel fallbacks |
| spec_app_dependency | PASS | PASS | spec crate remains detached from app crate at the Cargo dependency boundary |

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

<!-- arch-hotspot key=spec_runtime lines=3528 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3573 functions=48 -->
<!-- arch-hotspot key=provider_mod lines=376 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=355 functions=14 -->
<!-- arch-hotspot key=acp_manager lines=3391 functions=8 -->
<!-- arch-hotspot key=acpx_runtime lines=2946 functions=58 -->
<!-- arch-hotspot key=channel_registry lines=10222 functions=72 -->
<!-- arch-hotspot key=channel_config lines=9716 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6976 functions=147 -->
<!-- arch-hotspot key=channel_mod lines=1785 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=10833 functions=97 -->
<!-- arch-hotspot key=tools_mod lines=14810 functions=53 -->
<!-- arch-hotspot key=daemon_lib lines=6806 functions=224 -->
<!-- arch-hotspot key=onboard_cli lines=9723 functions=235 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
