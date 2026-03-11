# Alpha-Test Engineering Optimization Checklist (2026-03-11)

This checklist tracks architecture hardening and long-term sustainability work for `alpha-test`.

## Baseline Assessment Snapshot

- Crash evidence: macOS daemon abort trace captured at `~/Library/Logs/DiagnosticReports/loongclawd-8971dc537d3d8ba9-2026-03-11-084504.ips` (Wasmtime machports trap handler path).
- Complexity hotspots (lines/functions):
  - `crates/spec/src/spec_runtime.rs`: `2771/46`
  - `crates/spec/src/spec_execution.rs`: `1441/22`
  - `crates/app/src/provider/mod.rs`: `845/9`
  - `crates/app/src/memory/mod.rs`: `303/11`
- Reliability baseline (this branch):
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo test --workspace --all-features --locked`
  - `./scripts/stress_daemon_tests.sh`
- Latest verification run (`2026-03-11`): all gates passed, including
  - `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
  - `LOONGCLAW_STRESS_WASM_TRAPS_MODES=auto,false ./scripts/stress_daemon_tests.sh 1 default,2`

## Completed in Current Track

- [x] Wasm trap mode made platform-aware by default, with explicit env override (`LOONGCLAW_WASM_SIGNALS_BASED_TRAPS`).
- [x] Memory plane callsites migrated to abstraction constants/builders (`MEMORY_OP_*`, request builders, payload decoder).
- [x] Provider orchestration split into dedicated internal modules:
  - `crates/app/src/provider/model_selection.rs`
  - `crates/app/src/provider/payload_adaptation.rs`
- [x] `spec_runtime` Wasm runtime policy parsing moved into dedicated module:
  - `crates/spec/src/spec_runtime/wasm_runtime_policy.rs`
- [x] `spec_runtime` process stdio bridge path extracted into dedicated module:
  - `crates/spec/src/spec_runtime/process_stdio_bridge.rs`
- [x] `spec_runtime` HTTP JSON bridge path extracted into dedicated module:
  - `crates/spec/src/spec_runtime/http_json_bridge.rs`
- [x] `spec_runtime` Wasm cache internals extracted into dedicated module:
  - `crates/spec/src/spec_runtime/wasm_cache.rs`
- [x] `spec_execution` tool search logic extracted into dedicated module:
  - `crates/spec/src/spec_execution/tool_search.rs`
- [x] `spec_execution` security scan evaluation/runtime checks extracted into dedicated module:
  - `crates/spec/src/spec_execution/security_scan_eval.rs`
- [x] `spec_execution` security scan policy/profile loading and SIEM export pipeline extracted into dedicated module:
  - `crates/spec/src/spec_execution/security_scan_policy.rs`
- [x] `spec_execution` approval guard and risk-profile pipeline extracted into dedicated module:
  - `crates/spec/src/spec_execution/approval_policy.rs`
- [x] `spec_execution` bridge support policy checksum/sha256 and security profile canonicalization extracted into dedicated module:
  - `crates/spec/src/spec_execution/bridge_support_policy.rs`
- [x] `spec_execution` blocked-operation report assembly centralized via shared builder to remove duplicated return payload construction.
- [x] Architecture boundary checker added:
  - `scripts/check_architecture_boundaries.sh`
  - Task entries: `check:architecture`, `check:architecture:strict`
- [x] Daemon stress script supports optional trap-mode matrix (`auto|false|true`) via `LOONGCLAW_STRESS_WASM_TRAPS_MODES`.
- [x] Provider request gate/error policy extracted into dedicated module:
  - `crates/app/src/provider/error_policy.rs`

## Detailed Refactor Backlog

### P0 (Immediate, Stabilize Core Maintainability)

- [x] Split provider orchestration module (`crates/app/src/provider/mod.rs`) into bounded submodules.
  - Delivered slices:
    - `provider/model_selection.rs` (catalog fetch, ranking, retry)
    - `provider/payload_adaptation.rs` (payload-mode fallback and body shaping)
    - `provider/error_policy.rs` (error parsing and retry-next-model policy)
  - Acceptance:
    - public API signatures unchanged
    - `provider` tests remain green
    - architecture budget script still passes

- [x] Split bridge runtime internals from `crates/spec/src/spec_runtime.rs`.
  - Proposed slices:
    - `spec_runtime/wasm_cache.rs` (done)
    - `spec_runtime/process_stdio_bridge.rs` (done)
    - `spec_runtime/http_json_bridge.rs` (done)
  - Acceptance:
    - zero behavior drift on spec runtime tests
    - no reduction in audit fields
    - bridge strict-contract tests unchanged and green

- [x] Split `spec_execution` tool search path into dedicated submodule.
  - Delivered slice:
    - `spec_execution/tool_search.rs` (done)
  - Acceptance:
    - `OperationSpec::ToolSearch` behavior unchanged
    - full workspace tests green
    - architecture budget script still passes

- [x] Split `spec_execution` security finding/evaluation path into dedicated submodule.
  - Delivered slices:
    - `spec_execution/security_scan_eval.rs` (done)
    - `spec_execution/security_scan_policy.rs` (done)
    - `spec_execution/approval_policy.rs` (done)
    - `spec_execution/bridge_support_policy.rs` (done)
  - Acceptance:
    - security scan behavior unchanged under existing runtime tests
    - full workspace tests green
    - architecture budget script still passes

- [ ] Establish CI-visible architecture check stage (without changing workflow logic by default).
  - Local gate first: `task check:architecture:strict`
  - Acceptance:
    - command deterministic on macOS/Linux
    - no false positives in current codebase

### P1 (Medium-Term, Raise Abstraction Quality)

- [x] Introduce typed provider request context object to replace scattered primitive arguments.
  - Goal: reduce long argument lists in request functions and prevent argument-order mistakes.
  - Acceptance:
    - clippy clean
    - callsite argument count reduced in `request_*_with_model`

- [ ] Add focused property tests for parser/config boundaries.
  - Targets:
    - Wasm env parser aliases and invalid values
    - memory window payload tolerance and missing fields
  - Acceptance:
    - pre-fix regression seeds captured
    - deterministic and non-flaky tests

- [ ] Build a compact runtime resilience matrix doc.
  - Cover combinations of:
    - `LOONGCLAW_WASM_SIGNALS_BASED_TRAPS`
    - daemon `--test-threads`
    - retry/backoff policy key values
  - Acceptance:
    - matrix maps to executable commands
    - each row has pass/fail evidence link

### P2 (Long-Term, Sustainable Governance)

- [ ] Define architecture SLOs and monthly drift review.
  - SLO examples:
    - no hotspot file grows >10% month-over-month
    - no new cross-layer literal protocol strings outside adapters
  - Acceptance:
    - automated monthly report artifact committed under `docs/releases/`

- [ ] Introduce refactor budget policy per release.
  - Requirement: each feature release must pay down at least one hotspot metric.
  - Acceptance:
    - release checklist includes explicit budget item

## Verification Protocol for Each Refactor PR

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features --locked`
4. `./scripts/check_architecture_boundaries.sh`
5. `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
6. `./scripts/stress_daemon_tests.sh 3 default,2,1`
7. Optional trap matrix: `LOONGCLAW_STRESS_WASM_TRAPS_MODES=auto,false ./scripts/stress_daemon_tests.sh 3 default,2`

## Stop Conditions

- If any stress run reproduces daemon abort, stop feature work and prioritize runtime stability.
- If architecture strict gate fails, no further module growth until either:
  - hotspot is split, or
  - budget thresholds are explicitly adjusted with rationale.
