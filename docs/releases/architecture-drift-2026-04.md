# Architecture Drift Report 2026-04

## Summary
- Generated at: 2026-04-01T15:00:36Z
- Report month: `2026-04`
- Baseline report: docs/releases/architecture-drift-2026-03.md
- Hotspots tracked: 14
- Boundary checks tracked: 4
- SLO status: PASS

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3470 | 3600 | 130 | 65 | 65 | 0 | 100.0% | TIGHT | 3455 | 0.4% | PASS | 65 |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3679 | 3700 | 21 | 44 | 80 | 36 | 99.4% | TIGHT | 3547 | 3.7% | PASS | 43 |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 375 | 1000 | 625 | 10 | 20 | 10 | 50.0% | HEALTHY | 375 | 0.0% | PASS | 10 |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 356 | 650 | 294 | 14 | 16 | 2 | 87.5% | WATCH | 356 | 0.0% | PASS | 14 |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3391 | 3600 | 209 | 8 | 12 | 4 | 94.2% | WATCH | 3383 | 0.2% | PASS | 8 |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 2698 | 2800 | 102 | 56 | 65 | 9 | 96.4% | TIGHT | 2698 | 0.0% | PASS | 56 |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 10378 | 10500 | 122 | 89 | 90 | 1 | 98.9% | TIGHT | 9922 | 4.6% | PASS | 88 |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 9798 | 9800 | 2 | 90 | 90 | 0 | 100.0% | TIGHT | 9796 | 0.0% | PASS | 90 |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6976 | 7300 | 324 | 147 | 160 | 13 | 95.6% | TIGHT | 6936 | 0.6% | PASS | 146 |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1779 | 6400 | 4621 | 0 | 110 | 110 | 27.8% | HEALTHY | 1779 | 0.0% | PASS | 0 |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 10826 | 11200 | 374 | 100 | 120 | 20 | 96.7% | TIGHT | 10831 | -0.0% | PASS | 98 |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14764 | 15000 | 236 | 56 | 70 | 14 | 98.4% | TIGHT | 14472 | 2.0% | PASS | 54 |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 6340 | 6500 | 160 | 210 | 210 | 0 | 100.0% | TIGHT | 6324 | 0.3% | PASS | 210 |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9519 | 9800 | 281 | 228 | 250 | 22 | 97.1% | TIGHT | 9519 | 0.0% | PASS | 228 |
## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (99.4%), acpx_runtime (96.4%), channel_registry (98.9%), channel_config (100.0%), chat_runtime (95.6%), turn_coordinator (96.7%), tools_mod (98.4%), daemon_lib (100.0%), onboard_cli (97.1%)
- WATCH hotspots (>=85% and <95% of any tracked budget): memory_mod (87.5%), acp_manager (94.2%)
- Mixed-class hotspots (size plus operational density): chat_runtime, channel_mod, turn_coordinator

## Boundary Checks

| Check | Status | Previous Status | Detail |
|---|---|---|---|
| memory_literals | PASS | PASS | memory operation literals are centralized in crates/app/src/memory/* |
| provider_mod_helper_definitions | PASS | PASS | provider/mod.rs keeps payload, parse, and recovery helper implementations outside the top-level module |
| conversation_provider_optional_binding_roundtrip | PASS | PASS | conversation/runtime.rs translates explicit conversation bindings into provider bindings without optional-kernel roundtrips |
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

<!-- arch-hotspot key=spec_runtime lines=3470 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3679 functions=44 -->
<!-- arch-hotspot key=provider_mod lines=375 functions=10 -->
<!-- arch-hotspot key=memory_mod lines=356 functions=14 -->
<!-- arch-hotspot key=acp_manager lines=3391 functions=8 -->
<!-- arch-hotspot key=acpx_runtime lines=2698 functions=56 -->
<!-- arch-hotspot key=channel_registry lines=10378 functions=89 -->
<!-- arch-hotspot key=channel_config lines=9798 functions=90 -->
<!-- arch-hotspot key=chat_runtime lines=6976 functions=147 -->
<!-- arch-hotspot key=channel_mod lines=1779 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=10826 functions=100 -->
<!-- arch-hotspot key=tools_mod lines=14764 functions=56 -->
<!-- arch-hotspot key=daemon_lib lines=6340 functions=210 -->
<!-- arch-hotspot key=onboard_cli lines=9519 functions=228 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
