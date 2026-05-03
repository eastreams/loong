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
   - add `--host-hook <hook>`, `--host-action <action>`, and/or `--tui-surface <surface>` as needed
   - add `--capability <name>` when the package needs additive capability intent beyond the default connector baseline
2. inspect the generated smoke command from `plugins init`
3. run `loong plugins inventory`
4. run `loong plugins doctor`
5. probe with:
   - `loong plugins invoke-host-hook`
   - `loong plugins invoke-tui-surface`
6. execute runtime-managed trusted TUI surfaces with:
   - `loong plugins run-tui-surface`

If you are already inside the shell-first TUI, Loong now exposes matching live
operator routes:

- `/extensions`
- `/extensions <plugin-id>`
- `/extensions run <plugin-id> <surface>`

Current built-in shell-first surfaces:

- `command_palette`
- `settings_flow`
- `startup_onboarding`

Those live routes now use the runtime-managed trusted host lane. They still do
not imply an implicit in-process executor promise.

Custom lowercase trusted TUI surface identifiers are also valid. The built-in
three above only get richer first-party Loong affordances by default.

## Current Scaffold Boundary

Right now, trusted host scaffolds are intentionally limited to:

- runnable `process_stdio` packages
- explicit source-language selection
- additive capability declaration on top of the default `invoke_connector` baseline
- generated local runtime stub files
- bounded smoke probes plus runtime-managed trusted TUI execution, not implicit in-process execution

For generic endpoint-backed connector packages, the current public loop is:

1. scaffold the package:
   - `loong plugins init ... --bridge-kind http_json --endpoint <url> --connector-operation <name>`
2. inspect the generated probe command from `plugins init`
3. run `loong plugins inventory`
4. run `loong plugins doctor`
5. probe one real connector operation with:
   - `loong plugins invoke-connector-operation`

If you declare connector operations, keep `loong_connector_operations_json` and
`loong_connector_operation_specs_json` aligned so Loong can replace placeholder
probe guidance with real operation names.

## Reference Examples

The repository now keeps several checked-in example lanes you can diff against
directly.

Generic endpoint-backed connector packages:

- `examples/plugins-http/http-json-search/`

Governed native runtime extensions:

- `examples/plugins-process/native-extension-python/`
- `examples/plugins-process/native-extension-javascript/`
- `examples/plugins-process/native-extension-typescript/`
- `examples/plugins-process/native-extension-go/`
- `examples/plugins-process/native-extension-rust/`

Trusted-host extensions:

- `examples/plugins-process/native-extension-trusted-host-javascript/`
- `examples/plugins-process/native-extension-trusted-host-go/`
- `examples/plugins-process/native-extension-trusted-host-rust/`

Managed bridge reference package (reference-first today):

- `examples/plugins-process/channel-bridge-javascript/`

Managed bridge scaffold lane:

```bash
loong plugins init ./weixin-bridge \
  --plugin-id weixin-bridge \
  --bridge-kind process_stdio \
  --source-language js \
  --channel-id weixin \
  --transport-family wechat_clawbot_ilink_bridge \
  --target-contract 'weixin:<account>:contact:<id> | weixin:<account>:room:<id>'
```

Managed bridge smoke probe:

```bash
loong plugins invoke-channel-bridge-operation \
  --root ./weixin-bridge \
  --plugin-id weixin-bridge \
  --operation send_message \
  --payload '{"target":"weixin:contact:demo","text":"hello"}' \
  --allow-command node
```

When runtime plugins are enabled but `runtime_plugins.roots` is left empty,
Loong now auto-discovers the standard local roots first:

- `./.loong/extensions`
- `~/.loong/agent/extensions`

If the same `plugin_id` exists in both places, `.loong/extensions/` wins and
the global package becomes a shadowed duplicate on operator surfaces.

When that happens, Loong's operator surfaces should give you enough truth to
review the conflict without executing extension code first. The practical loop
is:

1. run `loong status` or `loong doctor --json`
2. inspect the winning project-local package path
3. inspect the shadowed global package path
4. compare the manifests with `git diff --no-index`

The shared discovery guidance now promotes those review commands directly on
`status`, and `doctor --json` carries the same conflict actions in the runtime
plugin inventory payload.

## Validation

Use [SDK Validator Contract](../design-docs/sdk-validator-contract.md) when you
need to understand the line between:

- artifact-shape validation
- doctor and setup readiness
- install or activation failures
- runtime policy denials

## Native extension quickstart

Today the shortest practical public authoring lane is a
manifest-first `process_stdio` package.

### 1. Scaffold the package

Python:

```bash
loong plugins init ./weather-python \
  --plugin-id weather-python \
  --provider-id weather \
  --connector-name weather-stdio \
  --bridge-kind process_stdio \
  --source-language py
```

JavaScript:

```bash
loong plugins init ./weather-js \
  --plugin-id weather-js \
  --provider-id weather \
  --connector-name weather-stdio \
  --bridge-kind process_stdio \
  --source-language js
```

TypeScript:

```bash
loong plugins init ./weather-ts \
  --plugin-id weather-ts \
  --provider-id weather \
  --connector-name weather-stdio \
  --bridge-kind process_stdio \
  --source-language ts
```

This writes:

- `loong.plugin.json`
- `README.md`
- a runnable `index.py`, `index.js`, or `index.ts` stub

### 2. Edit the manifest and runtime file

The scaffolded manifest already declares the native extension contract fields
that Loong inventories before execution.

It also declares explicit package capabilities. Today the scaffold defaults to
`InvokeConnector`, and operator surfaces should show that capability intent
before execution.

For the current public lane, the scaffold also declares:

- `loong_extension_family=governed_native_runtime_extension`
- `loong_extension_trust_lane=governed_sidecar`

For governed native packages, keep `loong_extension_method_specs_json` aligned
with each declared method so Loong can explain `extension/event`,
`extension/command`, and `extension/resource` with shared labels, summaries,
sample payloads, and operator hints.

When governed event identifiers are declared, keep
`loong_extension_event_specs_json` aligned too so Loong can explain what
`session_start` and future events actually mean.

The scaffold also reserves:

- `loong_extension_host_hooks_json=[]`
- `loong_extension_tui_surfaces_json=[]`

If you keep that field empty, the package stays on the current governed sidecar
lane.

If you declare one or more `--host-hook` values at scaffold time, Loong now
switches the package onto:

- `loong_extension_family=trusted_host_extension`
- `loong_extension_trust_lane=trusted_host`

and the smoke path changes from `invoke-extension` to:

- `loong plugins invoke-host-hook`

The current trusted host lane is read-only and starts with daemon-owned
lifecycle seams.

### Trusted host scaffold

JavaScript:

```bash
loong plugins init ./weather-host-js \
  --plugin-id weather-host-js \
  --provider-id weather \
  --connector-name weather-host-stdio \
  --bridge-kind process_stdio \
  --source-language js \
  --host-hook turn_start \
  --host-hook turn_end
```

Probe one declared hook:

```bash
loong plugins invoke-host-hook \
  --root "./weather-host-js" \
  --plugin-id weather-host-js \
  --hook turn_start \
  --payload '{"turn_id":"demo-turn"}' \
  --allow-command node
```

Automatic trusted-host dispatch currently covers:

- `turn_start`
- `turn_end`
- `session_start`
- `session_shutdown`

The current shell-first TUI lane ships richer first-party affordances for:

- `command_palette`
- `settings_flow`
- `startup_onboarding`

Trusted host packages can also declare additional lowercase identifiers such as
`sidebar_widget`. Those custom surfaces stay runnable through
`loong plugins run-tui-surface` and `/extensions run`, even when they do not
yet have dedicated first-party UI affordances.

When you want Loong to explain those surfaces well, keep
`loong_extension_tui_surface_specs_json` aligned with each declared surface's
label, summary, sample payload, and operator hint.

The same applies to trusted host hooks: keep
`loong_extension_host_hook_specs_json` aligned with each declared hook's label,
summary, sample payload, and operator hint.

If a trusted package declares host actions, keep
`loong_extension_host_action_specs_json` aligned with each action's label,
summary, sample payload, and operator hint too.

Scaffold them with one or more `--tui-surface` flags:

```bash
loong plugins init ./weather-host-ui \
  --plugin-id weather-host-ui \
  --provider-id weather \
  --connector-name weather-host-ui \
  --bridge-kind process_stdio \
  --source-language js \
  --host-hook turn_start \
  --tui-surface command_palette
```

These TUI declarations are now both contract-first and runtime-backed: Loong
inventories and validates them on the trusted host lane, and the same runtime
roots can dispatch them through the live trusted TUI executor.

You can still probe the declared TUI contract through the bounded bridge:

```bash
loong plugins invoke-tui-surface \
  --root "./weather-host-ui" \
  --plugin-id weather-host-ui \
  --tui-surface command_palette \
  --payload '{"query":":ext"}' \
  --allow-command node
```

When you want the runtime-managed path instead of the bounded authoring probe,
use:

```bash
loong plugins run-tui-surface \
  --config ./loong.toml \
  --plugin-id weather-host-ui \
  --tui-surface command_palette \
  --payload '{"query":":ext"}'
```

The scaffolded runtime file already handles a small starter surface:

- `extension/event`
- `extension/command`
- `extension/resource`

Replace it with your real implementation as the package becomes concrete.

### 3. Validate the package contract

```bash
loong plugins doctor --root "./weather-python" --profile sdk-release
```

### 4. Inspect the package truth

```bash
loong plugins inventory --root "./weather-python"
```

### 5. Smoke-test the extension entrypoint

```bash
loong plugins invoke-extension \
  --root "./weather-python" \
  --plugin-id weather-python \
  --method extension/event \
  --payload '{"event":"session_start"}' \
  --allow-command python3
```

For JavaScript, replace `python3` with `node`.

For TypeScript, scaffold with `--source-language ts` and use the same
`--allow-command node`; the template runs through
`node --experimental-strip-types index.ts`.

Go:

```bash
loong plugins init ./weather-go \
  --plugin-id weather-go \
  --provider-id weather \
  --connector-name weather-stdio \
  --bridge-kind process_stdio \
  --source-language go
```

Smoke-test:

```bash
loong plugins invoke-extension \
  --root "./weather-go" \
  --plugin-id weather-go \
  --method extension/event \
  --payload '{"event":"session_start"}' \
  --allow-command go
```

Rust:

```bash
loong plugins init ./weather-rust \
  --plugin-id weather-rust \
  --provider-id weather \
  --connector-name weather-stdio \
  --bridge-kind process_stdio \
  --source-language rs
```

Smoke-test:

```bash
loong plugins invoke-extension \
  --root "./weather-rust" \
  --plugin-id weather-rust \
  --method extension/event \
  --payload '{"event":"session_start"}' \
  --allow-command cargo
```

The first Rust smoke run may take longer because the scaffold uses
`cargo run --quiet --manifest-path Cargo.toml` behind the governed bridge.

This smoke path is explicit by design: local process execution only happens
when you pass the allowed command on the CLI.

## Auto-discovery locations

If you do not want to set `runtime_plugins.roots` explicitly, Loong now
auto-discovers runtime plugin packages from:

- `.loong/extensions/` — project-local
- `~/.loong/agent/extensions/` — global

This keeps the authoring lane open by default while preserving a Loong-native
directory contract.

If the same `plugin_id` exists in both places, `.loong/extensions/` wins and
the global package becomes a shadowed duplicate on operator surfaces.

When that happens, Loong's operator surfaces should give you enough truth to
review the conflict without executing extension code first. The practical loop
is:

1. run `loong status` or `loong doctor --json`
2. inspect the winning project-local package path
3. inspect the shadowed global package path
4. compare the manifests with `git diff --no-index`

The shared discovery guidance now promotes those review commands directly on
`status`, and `doctor --json` carries the same conflict actions in the runtime
plugin inventory payload.

## Supported runnable templates

| Language | `--source-language` | Scaffolded runtime files | Smoke `--allow-command` | Checked-in example |
|----------|----------------------|--------------------------|-------------------------|--------------------|
| Python | `py` | `index.py` | `python3` | `examples/plugins-process/native-extension-python/` |
| JavaScript | `js` | `index.js` | `node` | `examples/plugins-process/native-extension-javascript/` |
| TypeScript | `ts` | `index.ts` | `node` | `examples/plugins-process/native-extension-typescript/` |
| Go | `go` | `main.go` | `go` | `examples/plugins-process/native-extension-go/` |
| Rust | `rs` | `Cargo.toml`, `src/main.rs` | `cargo` | `examples/plugins-process/native-extension-rust/` |

## Reference example

The repository now also carries a minimal manifest-first example under:

- `examples/plugins-process/native-extension-python/`
- `examples/plugins-process/native-extension-javascript/`
- `examples/plugins-process/native-extension-typescript/`
- `examples/plugins-process/native-extension-go/`
- `examples/plugins-process/native-extension-rust/`

Use them when you want concrete `loong.plugin.json` packages plus runnable
entrypoints instead of starting from an empty package root.
