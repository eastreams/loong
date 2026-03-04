# ChumOS

ChumOS is a Rust-first Agentic OS foundation focused on:

- a stable kernel contract
- strict policy boundaries
- pluggable harness runtimes (Embedded Pi / ACP)
- connector-based bidirectional integrations
- composable vertical delivery packs
- layered execution planes with core/extension split
- unified plane-level audit evidence across execution layers
- test-first evolution for safety and stability

## Workspace Layout

- `crates/kernel`: core architecture contracts and execution kernel
- `crates/daemon`: minimal runnable daemon example wired to the kernel

## Core Design

`kernel` provides layered low-level planes:

1. `pack` (core): vertical pack manifest, semantic version validation, capability grants
2. `policy` (core): capability-token issuance, revocation, least-privilege authorization
3. `policy_ext` (extension): composable policy guards for environment or domain-specific denial rules
4. `harness` (core): dispatch broker for `EmbeddedPi` / `Acp` execution adapters
5. `runtime` (core + extension): core runtime adapters plus extension adapters that compose on top of core
6. `tool` (core + extension): minimal core tool substrate plus extension tool chains
7. `memory` (core + extension): minimal memory substrate plus semantic/vector enrichment extensions
8. `connector` (core + extension): legacy registry compatibility path plus core/extension connector plane
9. `audit` + `clock` (core): deterministic event timeline and audit sink abstraction
10. `integration` (control plane): provider/channel catalog, auto-provision planning, and hotfix support
11. `plugin_ir` (translation plane): canonical language-agnostic plugin IR with bridge-kind inference
12. `bootstrap` (activation execution plane): ready-plugin bootstrap execution policy (`applied/deferred`) and enforced gating
13. `architecture` + `awareness` (self-governance plane): immutable-core guard + deterministic self-snapshot
14. `kernel` (orchestration): enforces pack boundaries + policy checks + audit emission across all planes

Kernel orchestration also exposes explicit default-core selection APIs for each plane:

- `set_default_core_connector_adapter`
- `set_default_core_runtime_adapter`
- `set_default_core_tool_adapter`
- `set_default_core_memory_adapter`

## Core vs Extension Design Rule

- Core layer must stay minimal, deterministic, and auditable.
- Extension layer can be rich and fast-evolving, but must execute through core contracts.
- No extension bypasses pack capability boundaries or policy checks.
- New features should prefer adding extension adapters over mutating core contracts.

## Testing Strategy

The repository is intentionally test-first for architecture safety:

- Contract tests: pack validation, capability boundaries, token expiry/revocation
- Security tests: policy denial paths and extension denial paths
- Layer tests: runtime/tool/memory core+extension composition
- Integration-layer tests: connector core+extension composition with pack/policy boundaries
- Routing tests: harness kind-based auto routing (`EmbeddedPi` / `Acp`)
- Audit tests: token/task/connector lifecycle plus per-plane invocation evidence
- Schema tests: golden JSON assertion for key audit event contracts
- Property tests: capability-boundary invariants across generated combinations

Current status:

- `kernel` tests: 39 passing unit tests
- `daemon` tests: 37 passing unit tests
- `daemon` smoke: command-level demo + spec runner execution verified

Roadmap:

- `docs/roadmap.md`

## Quick Start

```bash
cargo test -p kernel
cargo test -p daemon
cargo run -p daemon
cargo run -p daemon -- run-task --objective "triage flaky tests" --payload '{"repo":"chumyin/ChumOS"}'
cargo run -p daemon -- invoke-connector --operation notify --payload '{"channel":"ops","message":"done"}'
cargo run -p daemon -- audit-demo
cargo run -p daemon -- init-spec --output examples/spec/runtime-extension.json
cargo run -p daemon -- run-spec --spec examples/spec/runtime-extension.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/auto-provider-hotplug.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/plugin-scan-hotplug.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/plugin-bridge-enforce.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/plugin-bootstrap-enforce.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/plugin-process-stdio-exec.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/tool-approval-per-call.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/self-awareness-guard.json --print-audit
cargo run -p daemon -- run-spec --spec examples/spec/plugin-wasm-security-scan.json --print-audit
```

The daemon crate now supports a JSON spec-driven path (`run-spec`) so you can execute
task/connector/runtime/tool/memory flows without changing Rust code.

Supported `operation.kind` values in spec:

- `task`
- `connector_legacy`
- `connector_core`
- `connector_extension`
- `runtime_core`
- `runtime_extension`
- `tool_core`
- `tool_extension`
- `memory_core`
- `memory_extension`

Reference spec file:

- `examples/spec/runtime-extension.json`
- `examples/spec/auto-provider-hotplug.json`
- `examples/spec/plugin-scan-hotplug.json`
- `examples/spec/plugin-bridge-enforce.json`
- `examples/spec/plugin-bootstrap-enforce.json`
- `examples/spec/plugin-process-stdio-exec.json`
- `examples/spec/tool-approval-per-call.json`
- `examples/policy/approval-medium-balanced.json`
- `examples/policy/security-scan-medium-balanced.json`
- `examples/spec/self-awareness-guard.json`
- `examples/spec/plugin-wasm-security-scan.json`
- `examples/plugins-wasm/secure_wasm_plugin.rs`
- `examples/plugins-wasm/secure_echo.wasm`

Spec supports autonomous provider/channel management:

- `auto_provision`: missing provider/channel planning and complete pack boundary update
- `hotfixes`: runtime provider/channel patch instructions (endpoint/version/enablement)
- `plugin_scan`: scan existing code files (any language text) for embedded plugin manifests,
  absorb them into catalog+pack, and hotplug immediately
- `bridge_support`: runtime bridge support matrix (`supported_bridges` +
  `supported_adapter_families`) with optional strict enforcement (`enforce_supported=true`) and
  optional integrity lock (`policy_version` + `expected_checksum` + `expected_sha256`) and
  optional local bridge runtime switches (`execute_process_stdio`, `execute_http_json`,
  `allowed_process_commands`, `enforce_execution_success`) and optional plugin security scan
  policy (`security_scan`, `security_scan.profile_path`)
- `bootstrap`: plugin bootstrap executor policy (per-bridge auto-apply switches, max task
  cap, and optional `enforce_ready_execution=true` hard gate)
- `self_awareness`: build a self-code snapshot (languages, plugin IR inventory, architecture guard)
  and optionally block execution when proposed mutations touch immutable-core paths
- `approval`: human approval gate for risky calls, with medium-balanced default policy and
  configurable modes (`disabled` / `medium_balanced` / `strict`), approval strategy
  (`per_call` / `one_time_full_access`), scope (`tool_calls` / `all_operations`), per-call
  allowlist (`approved_calls`), denylist (`denied_calls`), one-time full-access expiry/use limits
  (`one_time_full_access_expires_at_epoch_s`, `one_time_full_access_remaining_uses`), external
  risk profile loading (`risk_profile_path`), and optional inline risk-signal overrides
  (`high_risk_keywords`, `high_risk_tool_names`, `high_risk_payload_keys`)

`plugin_scan` now also emits plugin translation reports using canonical Plugin IR profiles
for multi-language bridge planning (`http_json`, `process_stdio`, `native_ffi`,
`wasm_component`, `mcp_server`).
It also emits plugin activation plans that evaluate each plugin against the current bridge
support matrix before hotplug absorb is applied, plus a `plugin_bootstrap_queue` of ready actions.
When bridge enforcement is disabled, unsupported plugins are skipped (not absorbed) while
ready plugins continue to hotplug. When bootstrap policy is enabled, only bootstrap-`applied`
plugins are absorbed into catalog+pack.
Multi-root `plugin_scan` is transactional: if any root is blocked by bridge/bootstrap guard,
no staged plugin absorb is committed.
Plugin scanner now processes files in deterministic sorted order so security/bootstrap behavior is stable.
`bootstrap.max_tasks` is enforced as a global budget across all scan roots in one run.
Translator-derived runtime metadata (`bridge_kind`, `adapter_family`, `entrypoint_hint`) is
automatically backfilled into absorbed plugin provider metadata when missing, so multi-language
plugins can still execute through a deterministic bridge path without hand-written bridge fields.
Connector invocation outcome now includes a normalized `bridge_execution` section to show exactly
how each hotplug provider will execute (`http_json`, `process_stdio`, `native_ffi`,
`wasm_component`, `mcp_server`).
When `execute_process_stdio=true` and command is allowlisted, bridge runtime can execute local
process plugins directly and attach structured runtime evidence in `bridge_execution.runtime`.
Default approval policy is medium-balanced: only high-risk tool calls (for example delete/remove/drop)
require human authorization; low-risk operations keep developer velocity.
Human authorization is flexible: either approve each call (`approval.approved_calls`) or grant
one-time full access (`one_time_full_access_granted=true`) for the current run.
`approval.denied_calls` always takes precedence over allowlist/full-access.
Risk modeling is profile-driven by default (`crates/daemon/config/approval-medium-balanced.json`);
for runtime customization, set `approval.risk_profile_path` to a JSON profile file. Inline
`high_risk_*` arrays are optional overlays for emergency customization, not the primary source.
Run report now emits `approval_guard` with `risk_level`, `risk_score`, `denylisted`,
`requires_human_approval`, and final approval decision.
When `bridge_support.security_scan.enabled=true`, run report emits `security_scan_report` with
plugin findings (`low`/`medium`/`high`) and optional hard block under `block_on_high=true`.
Security scan evaluates all activation-ready plugins before bootstrap budget filtering, so deferred
ready plugins are still risk-scanned in the same run.
When audit output is enabled (`--print-audit`), security scan also emits a typed
`SecurityScanEvaluated` event into `audit_events`, including finding counts, block reason, and
deduplicated finding categories for deterministic post-run governance.
Security scan defaults are profile-driven from
`crates/daemon/config/security-scan-medium-balanced.json`; for runtime customization, set
`bridge_support.security_scan.profile_path` to a JSON profile file.
WASM-focused checks include artifact path constraints (`allowed_path_prefixes`), module size cap
(`max_module_bytes`), SHA256 pin enforcement (`require_hash_pin` + `required_sha256_by_plugin`),
and import policy (`allow_wasi`, `blocked_import_prefixes`).
Scanner defaults skip noisy/build directories (`target`, `.git`, `node_modules`, `.venv`).

Plugin manifest embedding format (works in any language comment style):

- Start marker: `CHUMOS_PLUGIN_START`
- End marker: `CHUMOS_PLUGIN_END`
- Payload: JSON block matching fields:
  - `plugin_id`
  - `provider_id`
  - `connector_name`
  - `channel_id` (optional)
  - `endpoint` (optional)
  - `capabilities` (array of `Capability` enum values)
  - `metadata` (string map)

For deeper low-level architecture details, see `docs/layered-kernel-design.md`.
