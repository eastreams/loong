# Plugin Runtime Governance

This page centralizes plugin intake, activation, execution, and security controls.

## Discovery and Translation

- `plugin_scan` reads embedded manifests from source files (multi-language).
- translator emits canonical runtime profiles: `http_json`, `process_stdio`, `native_ffi`,
  `wasm_component`, `mcp_server`.
- activation planning evaluates each plugin against bridge support before absorb.

## Activation and Bootstrap

- `bridge_support.enforce_supported=true` blocks unsupported runtime profiles.
- when enforcement is disabled, unsupported plugins are skipped while ready plugins continue.
- `bootstrap` controls bridge-specific auto-apply and optional `enforce_ready_execution=true`.
- `bootstrap.max_tasks` is enforced as a global budget across all scan roots.
- multi-root scan/absorb is transactional: blocked roots prevent partial staged commits.

## Runtime Execution Controls

- `execute_http_json=true` enables active HTTP bridge execution with runtime evidence.
- `execute_process_stdio=true` + `allowed_process_commands` enables local process bridges.
- `bridge_execution` output captures normalized runtime path and evidence.

WASM runtime controls are policy-driven via `bridge_support.security_scan.runtime`:

- `allowed_path_prefixes` (required fail-closed guard)
- `max_component_bytes`
- optional `fuel_limit`

## Security Scan and Supply-Chain Gates

`bridge_support.security_scan` provides deterministic safety policy:

- severity findings (`low`/`medium`/`high`)
- optional hard block (`block_on_high=true`)
- profile loading via `profile_path`
- optional hash pin (`profile_sha256`) fail-closed
- optional signature verification (`profile_signature`, ed25519) fail-closed
- optional SIEM export (`siem_export`) with optional `fail_on_error=true`

WASM-focused static checks include:

- artifact path constraints
- module size cap
- SHA256 pin enforcement (`require_hash_pin`, `required_sha256_by_plugin`)
- import policy (`allow_wasi`, `blocked_import_prefixes`)

## Approval Guard

`approval` controls risk gating for tool-like operations:

- modes: `disabled`, `medium_balanced`, `strict`
- strategies: `per_call`, `one_time_full_access`
- scopes: `tool_calls`, `all_operations`
- denylist precedence over allowlist/full-access
- external risk profile (`risk_profile_path`) with optional inline emergency overlays

Run output includes `approval_guard` (`risk_level`, `risk_score`, decision rationale).

## Related Docs

- Manifest syntax: [Plugin Manifest Format](./plugin-manifest-format.md)
- Spec execution: [Spec Runner Reference](./spec-runner.md)
