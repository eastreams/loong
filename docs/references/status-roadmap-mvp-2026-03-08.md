# LoongClaw Status, Roadmap, and MVP Progress (2026-03-08)

Last updated: 2026-03-08

This document summarizes the latest deep optimization work, the current roadmap shape,
and MVP delivery progress against the current target scope.

## 1) Executive Summary

LoongClaw has moved from "feature presence" to "production-style gate discipline" in
the programmatic pressure lane.

Recent upgrades focus on:

- strict preflight validation before heavy benchmark execution
- structured baseline linting with deterministic issue taxonomy
- warning-aware gate policy (`fail-on-warnings`) for both lint and benchmark preflight
- modularized bridge runtime evidence and reduced test-file complexity
- complete validation matrix execution after each hardening iteration

In short:

- architecture direction is stable
- safety and regression gates are stronger and more explicit
- MVP core is usable now, with onboarding/distribution work still remaining

## 2) Deep Work Summary (Recent Completed Tracks)

## 2.1 Programmatic Pressure Gate Hardening

Implemented:

- schema fingerprint contract per `spec_run` scenario
- baseline threshold support for `expected_schema_fingerprint`
- strict enforcement behavior when fingerprint baseline is missing
- strict preflight fail-fast before running scenarios
- structured preflight output in benchmark report (`gate.preflight`)

Current command surface:

- `benchmark-programmatic-pressure`
- `benchmark-programmatic-pressure-lint`

Current gate policy controls:

- `--enforce-gate`
- `--fail-on-warnings` (lint command)
- `--preflight-fail-on-warnings` (benchmark preflight)

## 2.2 Baseline Lint as a Standalone Safety Lane

Implemented a dedicated static lint path for baseline quality checks (without running
pressure scenarios), including machine-readable report output.

Issue taxonomy currently includes:

- `duplicate_matrix_scenario_name`
- `missing_matrix_scenario_baseline_thresholds`
- `missing_spec_run_baseline_scenario`
- `missing_spec_run_schema_fingerprint`
- `unknown_baseline_scenario`
- `non_spec_run_schema_fingerprint_configured`

This makes baseline integrity auditable and automatable in CI pipelines.

## 2.3 Modularity and Technical Debt Reduction

Completed modularization:

- bridge protocol helpers extracted to dedicated include modules
- bridge runtime evidence path refactored to typed structs
- bridge runtime tests split into focused modules by transport/runtime lane

Result:

- lower coupling in spec runtime code paths
- easier future extension for protocol transports and bridge policies
- improved maintenance cost profile

## 2.4 Validation Discipline (Operational Quality)

Each major change cycle was validated with:

- `cargo fmt --all`
- daemon pressure benchmark test suite
- daemon full test suite
- workspace full test suite
- feature-slice compile matrix (no-default and representative channel/provider sets)
- pressure benchmark script and lint script
- negative tests for strict gate behavior (expected fail)

Current observed baseline after latest run:

- `loongclaw-kernel`: 41 tests passing
- `loongclaw-daemon`: 135 tests passing
- pressure benchmark gate: passing
- baseline lint gate: passing (clean baseline)

## 3) Roadmap Snapshot (Current Shape)

This section is a practical snapshot of `docs/roadmap.md`.

| Stage | Status | What it means now | Near-term focus |
|---|---|---|---|
| Stage 0: Kernel Contract Freeze | Done | Core boundary model is stable and test-proven | Keep contract drift at zero |
| Stage 1: Baseline Security & Governance | In Progress | Approval, policy, scan, signature, audit are functional | Key lifecycle and trust-anchor maturity |
| Stage 2: Safe Hotplug Runtime | In Progress | WASM runtime lane exists with guardrails | stronger runtime limits + rollback semantics |
| Stage 3: Autonomous Integration Expansion | In Progress | Protocol/routing/orchestration foundation is in place | versioning + reconciliation + richer adapters |
| Stage 4: Community Plugin Supply Chain | Planned | Trust tier and provenance model defined | packaging/signing pipeline |
| Stage 5: Vertical Pack Productization | Planned | Productization model defined | 15-minute vertical pack workflow |
| Stage M: End-User MVP Product Layer | In Progress | MVP command surface is usable | onboarding/distribution and first-run UX |

Strategic read:

- kernel and governance foundation is strong enough for controlled expansion
- major remaining gap is not core correctness, but productization and distribution

## 4) MVP Progress Against Target Scope

Scope baseline (current target) and assessed status:

| MVP Scope Item | Status | Notes |
|---|---|---|
| 1. Rust lightweight daemon core | Done | `loongclawd` command surface and layered runtime are operational |
| 2. Channels: CLI / Telegram / Feishu | Done | command paths and adapters are in place (`chat`, `telegram`, `feishu`) |
| 3. OpenAI-compatible + Volcengine custom provider | Mostly Done | feature support and config path exist; hardening remains |
| 4. TOML config format | Done | `setup` flow and config modules active |
| 5. Shell tool | Done | `shell.exec` available under policy |
| 6. File tool | Done | `file.read` / `file.write` available under policy |
| 7. SQLite memory + sliding window | Done | conversation memory and windowed retrieval are active |
| 8. Minimal install (prebuilt + setup) | Partial | `setup` exists; prebuilt binary distribution still pending |
| 9. Beginner-friendly onboarding | Partial | quickstart/setup exists; guided doctor-style workflow still missing |

Estimated MVP completion (engineering view): about 75% to 80%.

Main blockers to "MVP ready for broad non-dev users":

- prebuilt multi-platform binary delivery pipeline
- guided onboarding and diagnostics (`doctor`-like UX)
- provider adapter hardening to reduce first-run friction

## 5) Current Safety, Robustness, and Extensibility Posture

## 5.1 Safety

Strengths:

- policy-gated execution model
- strict preflight + lint gates for pressure baseline integrity
- explicit fail-fast behavior on critical baseline defects
- deterministic structured issue reporting

Remaining:

- further trust lifecycle management for signatures/keys
- broader external SIEM transport lanes

## 5.2 Robustness

Strengths:

- extensive regression matrix and negative-path validation
- scenario-level and baseline-level gate separation
- reduced large-file coupling through modularization

Remaining:

- deeper chaos-style runtime failure injection around hot-reload and recovery
- tighter SLO-style threshold governance for noisy environments

## 5.3 Extensibility and Sustainability

Strengths:

- feature-flag modular packaging is already effective
- protocol and bridge abstractions support continued transport expansion
- typed issue model provides stable interface for CI/reporting integrations

Remaining:

- connector contract versioning
- idempotent auto-provision and reconciliation maturity

## 6) Recommended Next Iteration (Actionable)

Priority order:

1. Add a consolidated gate report command that merges benchmark + lint + feature matrix
   into a single release artifact.
2. Introduce baseline policy profiles (`strict`, `balanced`, `warning-tolerant`) to avoid
   ad-hoc flag combinations.
3. Implement onboarding diagnostics command (`doctor`) for MVP Stage M completion.
4. Start prebuilt binary packaging lane for macOS/Linux/Windows with checksum publish.

Definition of done for next checkpoint:

- one-command gate report artifact for release decisions
- onboarding diagnostics command available
- first prebuilt release candidate available for at least one platform
