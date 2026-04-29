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

### Workflow or flow asset

Best fit when the behavior is more structured than prompt guidance and belongs
closer to reusable orchestration.

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

The scaffold also reserves:

- `loong_extension_host_hooks_json=[]`

That field is for future `trusted_host_extension` packages on the
`trusted_host` trust lane. The current governed sidecar lane inventories the
declaration, but does not execute host hooks.

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
