# Architecture Drift Report 2026-04

This report is a repository maintenance artifact for architecture-governance and
release review. It is not part of the primary public release trail.

## Route By Audience

| If you are trying to... | Start here |
| --- | --- |
| read public release history | the top-level `../vX.Y.Z*.md` and `../*-announcement.md` files |
| inspect architecture-maintenance and release-governance evidence | this report |
| understand the release-support file boundary | [`README.md`](README.md) |

## Read This File When

- you are reviewing release-support architecture evidence for `2026-04`
- you need the generated hotspot and boundary-check snapshot behind release
  governance
- you are validating whether release-support automation still matches the
  repository's current architecture boundaries

## Summary
- Generated at: 2026-04-19T08:28:03Z
- Report month: `2026-04`
- Baseline report: docs/releases/support/architecture-drift-2026-03.md
- Hotspots tracked: 14
- Boundary checks tracked: 5
- SLO status: FAIL

## Hotspot Metrics

| Key | Classes | File | Lines | Max Lines | Line Headroom | Functions | Max Functions | Fn Headroom | Peak Usage | Pressure | Prev Lines | Line Growth | Growth SLO | Prev Functions |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---|---:|
| spec_runtime | `foundation` | `crates/spec/src/spec_runtime.rs` | 3513 | 3600 | 87 | 65 | 65 | 0 | 100.0% | TIGHT | 3455 | 1.7% | PASS | 65 |
| spec_execution | `foundation` | `crates/spec/src/spec_execution.rs` | 3574 | 3700 | 126 | 48 | 80 | 32 | 96.6% | TIGHT | 3547 | 0.8% | PASS | 43 |
| provider_mod | `foundation` | `crates/app/src/provider/mod.rs` | 416 | 1000 | 584 | 13 | 20 | 7 | 65.0% | HEALTHY | 375 | 10.9% | BREACH | 10 |
| memory_mod | `foundation` | `crates/app/src/memory/mod.rs` | 456 | 650 | 194 | 12 | 16 | 4 | 75.0% | HEALTHY | 356 | 28.1% | BREACH | 14 |
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 2932 | 3600 | 668 | 0 | 12 | 12 | 81.4% | HEALTHY | 3383 | -13.3% | PASS | 8 |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 1776 | 2800 | 1024 | 7 | 65 | 58 | 63.4% | HEALTHY | 2698 | -34.2% | PASS | 56 |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 8906 | 10500 | 1594 | 54 | 90 | 36 | 84.8% | HEALTHY | 9922 | -10.2% | PASS | 88 |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 8260 | 9800 | 1540 | 0 | 90 | 90 | 84.3% | HEALTHY | 9796 | -15.7% | PASS | 90 |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6137 | 7300 | 1163 | 58 | 160 | 102 | 84.1% | HEALTHY | 6936 | -11.5% | PASS | 146 |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 2111 | 6400 | 4289 | 0 | 110 | 110 | 33.0% | HEALTHY | 1779 | 18.7% | BREACH | 0 |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 9529 | 11200 | 1671 | 50 | 120 | 70 | 85.1% | WATCH | 10831 | -12.0% | PASS | 98 |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14625 | 15000 | 375 | 45 | 70 | 25 | 97.5% | TIGHT | 14472 | 1.1% | PASS | 54 |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 4877 | 6500 | 1623 | 162 | 210 | 48 | 77.1% | HEALTHY | 6324 | -22.9% | PASS | 210 |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 5705 | 9800 | 4095 | 206 | 250 | 44 | 82.4% | HEALTHY | 9519 | -40.1% | PASS | 228 |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), tools_mod (97.5%)
- WATCH hotspots (>=85% and <95% of any tracked budget): turn_coordinator (85.1%)
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
- Release checklist budget field lives in `docs/releases/support/TEMPLATE.md`.
- Rule: each release must name at least one hotspot metric paid down or explicitly state why no paydown happened.

## Detail Links
- [Architecture gate](../../../scripts/check_architecture_boundaries.sh)
- [Release template](TEMPLATE.md)
- [CI workflow](../../../.github/workflows/ci.yml)

## Do Not Use This File For

- public release-history reading that should start from `vX.Y.Z*.md`,
  `*-announcement.md`, `CHANGELOG.md`, or GitHub Releases
- temporary maintainer scratch notes or architecture experiments that should
  live outside the tracked release-doc path
- backlog planning packages that do not belong in the OSS repository

<!-- arch-hotspot key=spec_runtime lines=3513 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3574 functions=48 -->
<!-- arch-hotspot key=provider_mod lines=416 functions=13 -->
<!-- arch-hotspot key=memory_mod lines=456 functions=12 -->
<!-- arch-hotspot key=acp_manager lines=2932 functions=0 -->
<!-- arch-hotspot key=acpx_runtime lines=1776 functions=7 -->
<!-- arch-hotspot key=channel_registry lines=8906 functions=54 -->
<!-- arch-hotspot key=channel_config lines=8260 functions=0 -->
<!-- arch-hotspot key=chat_runtime lines=6137 functions=58 -->
<!-- arch-hotspot key=channel_mod lines=2111 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=9529 functions=50 -->
<!-- arch-hotspot key=tools_mod lines=14625 functions=45 -->
<!-- arch-hotspot key=daemon_lib lines=4877 functions=162 -->
<!-- arch-hotspot key=onboard_cli lines=5705 functions=206 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
