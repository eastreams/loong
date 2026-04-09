# Architecture Drift Report 2026-04

## Summary
- Generated at: 2026-04-12T09:43:24Z
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
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 409 | 1000 | 591 | 11 | 20 | 9 | 55.0% | HEALTHY | 375 | 9.1% | PASS | 10 |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 456 | 650 | 194 | 16 | 16 | 0 | 100.0% | TIGHT | 356 | 28.1% | BREACH | 14 |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3476 | 3600 | 124 | 12 | 12 | 0 | 100.0% | TIGHT | 3383 | 2.7% | PASS | 8 |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2641 | 2800 | 159 | 49 | 65 | 16 | 94.3% | WATCH | 2698 | -2.1% | PASS | 56 |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9449 | 10500 | 1051 | 72 | 90 | 18 | 90.0% | WATCH | 9922 | -4.8% | PASS | 88 |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9684 | 9800 | 116 | 87 | 90 | 3 | 98.8% | TIGHT | 9796 | -1.1% | PASS | 90 |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6594 | 7300 | 706 | 95 | 160 | 65 | 90.3% | WATCH | 6936 | -4.9% | PASS | 146 |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1836 | 6400 | 4564 | 0 | 110 | 110 | 28.7% | HEALTHY | 1779 | 3.2% | PASS | 0 |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 10725 | 11200 | 475 | 87 | 120 | 33 | 95.8% | TIGHT | 10831 | -1.0% | PASS | 98 |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14609 | 15000 | 391 | 53 | 70 | 17 | 97.4% | TIGHT | 14472 | 0.9% | PASS | 54 |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6474 | 6500 | 26 | 198 | 210 | 12 | 99.6% | TIGHT | 6324 | 2.4% | PASS | 210 |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9787 | 9800 | 13 | 237 | 250 | 13 | 99.9% | TIGHT | 9519 | 2.8% | PASS | 228 |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), memory_mod (100.0%), acp_manager (100.0%), channel_config (98.8%), turn_coordinator (95.8%), tools_mod (97.4%), daemon_lib (99.6%), onboard_cli (99.9%)
- WATCH hotspots (>=85% and <95% of any tracked budget): acpx_runtime (94.3%), channel_registry (90.0%), chat_runtime (90.3%)
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
- [Release template](support/README.md)
- [CI workflow](../../.github/workflows/ci.yml)

<!-- arch-hotspot key=spec_runtime lines=3528 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3573 functions=48 -->
<!-- arch-hotspot key=provider_mod lines=409 functions=11 -->
<!-- arch-hotspot key=memory_mod lines=456 functions=16 -->
<!-- arch-hotspot key=acp_manager lines=3476 functions=12 -->
<!-- arch-hotspot key=acpx_runtime lines=2641 functions=49 -->
<!-- arch-hotspot key=channel_registry lines=9449 functions=72 -->
<!-- arch-hotspot key=channel_config lines=9684 functions=87 -->
<!-- arch-hotspot key=chat_runtime lines=6594 functions=95 -->
<!-- arch-hotspot key=channel_mod lines=1836 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=10725 functions=87 -->
<!-- arch-hotspot key=tools_mod lines=14609 functions=53 -->
<!-- arch-hotspot key=daemon_lib lines=6474 functions=198 -->
<!-- arch-hotspot key=onboard_cli lines=9787 functions=237 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
