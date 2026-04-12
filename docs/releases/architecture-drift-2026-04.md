# Architecture Drift Report 2026-04

## Summary
- Generated at: 2026-04-12T08:48:35Z
- Report month: `2026-04`
- Baseline report: none
- Hotspots tracked: 14
- Boundary checks tracked: 5
- SLO status: PASS

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3528 | 3600 | 72 | 65 | 65 | 0 | 100.0% | TIGHT |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3573 | 3700 | 127 | 48 | 80 | 32 | 96.6% | TIGHT |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 409 | 1000 | 591 | 11 | 20 | 9 | 55.0% | HEALTHY |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 456 | 650 | 194 | 16 | 16 | 0 | 100.0% | TIGHT |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 2871 | 3600 | 729 | 0 | 12 | 12 | 79.8% | HEALTHY |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 1776 | 2800 | 1024 | 7 | 65 | 58 | 63.4% | HEALTHY |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9449 | 10500 | 1051 | 72 | 90 | 18 | 90.0% | WATCH |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9684 | 9800 | 116 | 87 | 90 | 3 | 98.8% | TIGHT |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6571 | 7300 | 729 | 94 | 160 | 66 | 90.0% | WATCH |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1836 | 6400 | 4564 | 0 | 110 | 110 | 28.7% | HEALTHY |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 8408 | 11200 | 2792 | 36 | 120 | 84 | 75.1% | HEALTHY |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14731 | 15000 | 269 | 60 | 70 | 10 | 98.2% | TIGHT |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6466 | 6500 | 34 | 198 | 210 | 12 | 99.5% | TIGHT |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9787 | 9800 | 13 | 237 | 250 | 13 | 99.9% | TIGHT |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), memory_mod (100.0%), channel_config (98.8%), tools_mod (98.2%), daemon_lib (99.5%), onboard_cli (99.9%)
- WATCH hotspots (>=85% and <95% of any tracked budget): channel_registry (90.0%), chat_runtime (90.0%)
- Mixed-class hotspots (size plus operational density): chat_runtime, channel_mod, turn_coordinator

## Boundary Checks

| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | n/a | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | n/a | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |
| conversation_provider_optional_binding_roundtrip | PASS | n/a | conversation/runtime.rs translates explicit conversation bindings into provider bindings without optional-kernel roundtrips |
| conversation_app_dispatcher_optional_kernel_context | PASS | n/a | conversation app-tool dispatcher approval hooks stay binding-based without optional kernel fallbacks |
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

<!-- arch-hotspot key=spec_runtime lines=3528 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3573 functions=48 -->
<!-- arch-hotspot key=provider_mod lines=409 functions=11 -->
<!-- arch-hotspot key=memory_mod lines=456 functions=16 -->
<!-- arch-hotspot key=acp_manager lines=2871 functions=0 -->
<!-- arch-hotspot key=acpx_runtime lines=1776 functions=7 -->
<!-- arch-hotspot key=channel_registry lines=9449 functions=72 -->
<!-- arch-hotspot key=channel_config lines=9684 functions=87 -->
<!-- arch-hotspot key=chat_runtime lines=6571 functions=94 -->
<!-- arch-hotspot key=channel_mod lines=1836 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=8408 functions=36 -->
<!-- arch-hotspot key=tools_mod lines=14731 functions=60 -->
<!-- arch-hotspot key=daemon_lib lines=6466 functions=198 -->
<!-- arch-hotspot key=onboard_cli lines=9787 functions=237 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
