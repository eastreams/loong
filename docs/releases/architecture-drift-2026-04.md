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
- Generated at: 2026-04-13T04:06:52Z
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
| acp_manager | `operational_density` | `crates/app/src/acp/manager.rs` | 3092 | 3600 | 508 | 0 | 12 | 12 | 85.9% | WATCH |
| acpx_runtime | `operational_density` | `crates/app/src/acp/acpx.rs` | 1775 | 2800 | 1025 | 7 | 65 | 58 | 63.4% | HEALTHY |
| channel_registry | `structural_size` | `crates/app/src/channel/registry.rs` | 9449 | 10500 | 1051 | 72 | 90 | 18 | 90.0% | WATCH |
| channel_config | `structural_size` | `crates/app/src/config/channels.rs` | 8697 | 9800 | 1103 | 17 | 90 | 73 | 88.7% | WATCH |
| chat_runtime | `structural_size,operational_density` | `crates/app/src/chat.rs` | 6594 | 7300 | 706 | 95 | 160 | 65 | 90.3% | WATCH |
| channel_mod | `structural_size,operational_density` | `crates/app/src/channel/mod.rs` | 1836 | 6400 | 4564 | 0 | 110 | 110 | 28.7% | HEALTHY |
| turn_coordinator | `structural_size,operational_density` | `crates/app/src/conversation/turn_coordinator.rs` | 8424 | 11200 | 2776 | 36 | 120 | 84 | 75.2% | HEALTHY |
| tools_mod | `structural_size` | `crates/app/src/tools/mod.rs` | 14204 | 15000 | 796 | 42 | 70 | 28 | 94.7% | WATCH |
| daemon_lib | `structural_size` | `crates/daemon/src/lib.rs` | 5637 | 6500 | 863 | 174 | 210 | 36 | 86.7% | WATCH |
| onboard_cli | `structural_size` | `crates/daemon/src/onboard_cli.rs` | 9202 | 9800 | 598 | 205 | 250 | 45 | 93.9% | WATCH |

## Prioritization Signals
- BREACH hotspots (>100% of any tracked budget): none
- TIGHT hotspots (>=95% of any tracked budget): spec_runtime (100.0%), spec_execution (96.6%), memory_mod (100.0%)
- WATCH hotspots (>=85% and <95% of any tracked budget): acp_manager (85.9%), channel_registry (90.0%), channel_config (88.7%), chat_runtime (90.3%), tools_mod (94.7%), daemon_lib (86.7%), onboard_cli (93.9%)
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
- Release checklist budget field lives in `docs/releases/support/TEMPLATE.md`.
- Rule: each release must name at least one hotspot metric paid down or explicitly state why no paydown happened.

## Detail Links
- [Architecture gate](../../scripts/check_architecture_boundaries.sh)
- [Release template](support/TEMPLATE.md)
- [CI workflow](../../.github/workflows/ci.yml)

## Do Not Use This File For

- public release-history reading that should start from `vX.Y.Z*.md`,
  `*-announcement.md`, `CHANGELOG.md`, or GitHub Releases
- temporary maintainer scratch notes or architecture experiments that should
  live outside the tracked release-doc path
- backlog planning packages that do not belong in the OSS repository

<!-- arch-hotspot key=spec_runtime lines=3528 functions=65 -->
<!-- arch-hotspot key=spec_execution lines=3573 functions=48 -->
<!-- arch-hotspot key=provider_mod lines=409 functions=11 -->
<!-- arch-hotspot key=memory_mod lines=456 functions=16 -->
<!-- arch-hotspot key=acp_manager lines=3092 functions=0 -->
<!-- arch-hotspot key=acpx_runtime lines=1775 functions=7 -->
<!-- arch-hotspot key=channel_registry lines=9449 functions=72 -->
<!-- arch-hotspot key=channel_config lines=8697 functions=17 -->
<!-- arch-hotspot key=chat_runtime lines=6594 functions=95 -->
<!-- arch-hotspot key=channel_mod lines=1836 functions=0 -->
<!-- arch-hotspot key=turn_coordinator lines=8424 functions=36 -->
<!-- arch-hotspot key=tools_mod lines=14204 functions=42 -->
<!-- arch-hotspot key=daemon_lib lines=5637 functions=174 -->
<!-- arch-hotspot key=onboard_cli lines=9202 functions=205 -->
<!-- arch-boundary key=memory_literals status=PASS -->
<!-- arch-boundary key=provider_mod_helper_definitions status=PASS -->
<!-- arch-boundary key=conversation_provider_optional_binding_roundtrip status=PASS -->
<!-- arch-boundary key=conversation_app_dispatcher_optional_kernel_context status=PASS -->
<!-- arch-boundary key=spec_app_dependency status=PASS -->
