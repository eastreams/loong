# Spec Runner Reference

`loongclawd run-spec` executes declarative JSON specs end-to-end without changing Rust code.

## Supported Operation Kinds

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
- `tool_search`
- `programmatic_tool_call`

## Common Spec Sections

- `pack`: pack boundary, connector allowlist, granted capabilities.
- `agent_id` / `ttl_s`: execution identity and token lifetime.
- `approval`: human approval strategy and risk-profile behavior.
- `plugin_scan`: discover plugin manifests from code files.
- `bridge_support`: bridge support matrix and runtime switches.
- `bootstrap`: plugin activation policy and ready-task gating.
- `self_awareness`: deterministic codebase snapshot + architecture guard.
- `auto_provision`: missing provider/channel reconciliation.
- `hotfixes`: runtime endpoint/config patching.
- `operation`: concrete execution target.

## Quick Commands

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/runtime-extension.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/tool-search.json --print-audit
cargo run -p loongclaw-daemon --bin loongclawd -- run-spec --spec examples/spec/programmatic-tool-call.json --print-audit
```

## Key Example Specs

- `examples/spec/runtime-extension.json`
- `examples/spec/auto-provider-hotplug.json`
- `examples/spec/plugin-scan-hotplug.json`
- `examples/spec/plugin-bridge-enforce.json`
- `examples/spec/plugin-bootstrap-enforce.json`
- `examples/spec/plugin-process-stdio-exec.json`
- `examples/spec/tool-search.json`
- `examples/spec/programmatic-tool-call.json`
- `examples/spec/tool-approval-per-call.json`
- `examples/spec/self-awareness-guard.json`
- `examples/spec/plugin-wasm-security-scan.json`

## Related Docs

- Plugin governance details: [Plugin Runtime Governance](./plugin-runtime-governance.md)
- Programmatic orchestration details: [Programmatic Tool Call](./programmatic-tool-call.md)
- Architecture model: [Layered Kernel Design](../design-docs/layered-kernel-design.md)
