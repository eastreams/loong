# External Authoring Quickstart

Use this guide when you want the shortest practical summary of what Loong
expects external authors to build.

## Read This First

- [External Authoring Contract](../design-docs/external-authoring-contract.md)
- [SDK Validator Contract](../design-docs/sdk-validator-contract.md)
- [SDK Stability Policy](../design-docs/sdk-stability-policy.md)

## Public Stance

Loong's public SDK is contract-first and artifact-first.

Do not assume the stable public surface is:

- internal `crates/app` helper layout
- internal registries
- repository-only helper functions

Instead, the public surface is moving toward:

- package metadata
- package layout
- setup semantics
- validation
- controlled runtime lanes
- install, inspect, and audit behavior

## Which Family Fits?

### Managed skill

Best fit when the capability is reusable procedural guidance and should stay
installable and inspectable.

### Governed plugin package

Best fit when the capability needs a runtime lane, setup metadata, and explicit
ownership intent.

### Trusted host extension

Best fit when the capability needs declared host hooks or shell-first TUI
surfaces under an explicit trusted lane.

### Workflow or flow asset

Best fit when the behavior is more structured than prompt guidance and belongs
closer to reusable orchestration.

## Current Fast Path

For trusted host extensions, the shortest current loop is:

1. scaffold the package:
   - `loong plugins init ... --bridge-kind process_stdio --source-language <lang>`
   - add `--host-hook <hook>` and/or `--tui-surface <surface>` as needed
2. inspect the generated smoke command from `plugins init`
3. run `loong plugins inventory`
4. run `loong plugins doctor`
5. probe with:
   - `loong plugins invoke-host-hook`
   - `loong plugins invoke-tui-surface`

If you are already inside the shell-first TUI, Loong now exposes matching live
operator routes:

- `/extensions`
- `/extensions <plugin-id>`
- `/extensions run <plugin-id> <surface>`

Current live shell-first surfaces:

- `command_palette`
- `settings_flow`
- `startup_onboarding`

Those live routes are bounded probe surfaces, not a general-purpose in-process
executor promise.

## Current Scaffold Boundary

Right now, trusted host scaffolds are intentionally limited to:

- runnable `process_stdio` packages
- explicit source-language selection
- generated local runtime stub files
- bounded smoke probes, not implicit live execution

## Reference Example

The repository now keeps one checked-in trusted-host example package you can
diff against directly:

- `examples/plugins-process/native-extension-trusted-host-javascript/`

When runtime plugins are enabled but `runtime_plugins.roots` is left empty,
Loong now auto-discovers the standard local roots first:

- `./.loong/extensions`
- `~/.loong/agent/extensions`

## Validation

Use [SDK Validator Contract](../design-docs/sdk-validator-contract.md) when you
need to understand the line between:

- artifact-shape validation
- doctor and setup readiness
- install or activation failures
- runtime policy denials
