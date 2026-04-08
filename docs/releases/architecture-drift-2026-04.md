# Architecture Drift Report 2026-04

## Summary
- Generated at: 2026-04-08T16:50:04Z
- Report month: `2026-04`
- Baseline report: docs/releases/architecture-drift-2026-03.md
- Hotspots tracked: 14
- Boundary checks tracked: 5
- SLO status: FAIL

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3528 | 3600 | 72 | 65 | 65 | 0 | 100.0% | TIGHT | 3455 | 2.1% | PASS | 65 |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3573 | 3700 | 127 | 48 | 80 | 32 | 96.6% | TIGHT | 3547 | 0.7% | PASS | 43 |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 378 | 1000 | 622 | 10 | 20 | 10 | 50.0% | HEALTHY | 375 | 0.8% | PASS | 10 |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 456 | 650 | 194 | 16 | 16 | 0 | 100.0% | TIGHT | 356 | 28.1% | BREACH | 14 |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3476 | 3600 | 124 | 12 | 12 | 0 | 100.0% | TIGHT | 3383 | 2.7% | PASS | 8 |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2800 | 2800 | 0 | 56 | 65 | 9 | 100.0% | TIGHT | 2698 | 3.8% | PASS | 56 |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9450 | 10500 | 1050 | 72 | 90 | 18 | 90.0% | WATCH | 9922 | -4.8% | PASS | 88 |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9716 | 9800 | 84 | 90 | 90 | 0 | 100.0% | TIGHT | 9796 | -0.8% | PASS | 90 |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6848 | 7300 | 452 | 123 | 160 | 37 | 93.8% | WATCH | 6936 | -1.3% | PASS | 146 |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1786 | 6400 | 4614 | 0 | 110 | 110 | 27.9% | HEALTHY | 1779 | 0.4% | PASS | 0 |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 10094 | 11200 | 1106 | 83 | 120 | 37 | 90.1% | WATCH | 10831 | -6.8% | PASS | 98 |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14999 | 15000 | 1 | 55 | 70 | 15 | 100.0% | TIGHT | 14472 | 3.6% | PASS | 54 |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6485 | 6500 | 15 | 201 | 210 | 9 | 99.8% | TIGHT | 6324 | 2.5% | PASS | 210 |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9787 | 9800 | 13 | 237 | 250 | 13 | 99.9% | TIGHT | 9519 | 2.8% | PASS | 228 |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), memory_mod (100.0%), acp_manager (100.0%), acpx_runtime (100.0%), channel_config (100.0%), tools_mod (100.0%), daemon_lib (99.8%), onboard_cli (99.9%)
- WATCH hotspots (>=85% and <95% of any tracked budget): channel_registry (90.0%), chat_runtime (93.8%), turn_coordinator (90.1%)
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
- Hotspot growth SLO (>10% month-over-month): FAIL
- Boundary ownership SLO (helpers stay behind their module boundaries): PASS
- Overall architecture SLO status: FAIL

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
<!-- arch-hotspot key=provider_mod lines=378 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=456 functions=16 -->
<!-- arch-hotspot key=acp_manager lines=3476 functions=12 -->
<!-- arch-hotspot key=acpx_runtime lines=2800 functions=56 -->
<!-- arch-hotspot key=channel_registry lines=9450 functions=72 -->
<!-- arch-hotspot key=channel_config lines=9716 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6848 functions=123 -->
<!-- arch-hotspot key=channel_mod lines=1786 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=10094 functions=83 -->
<!-- arch-hotspot key=tools_mod lines=14999 functions=55 -->
<!-- arch-hotspot key=daemon_lib lines=6485 functions=201 -->
<!-- arch-hotspot key=onboard_cli lines=9787 functions=237 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
