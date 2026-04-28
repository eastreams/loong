# SDK Compatibility Matrix

This matrix summarizes the current maturity posture of the main SDK-adjacent
surfaces.

## Layer Matrix

| Layer | Current posture |
|-------|-----------------|
| Product capability surface | Stable to Additive |
| Internal integration SDK | Internal |
| External authoring contract | Additive moving toward Stable |
| Capability promotion contract | Additive |
| Live promotion executor | Experimental |
| Learning or ranking layers | Experimental |

## Asset Family Matrix

| Asset family | Current reading |
|--------------|-----------------|
| Managed skills | Most practical public capability family today |
| Manifest-first plugin packages | Active design direction with strong architectural contract |
| Workflow / flow assets | Strategically important, less concretely packaged today |
| Runtime-capability candidate artifacts | Shipped governance artifact family |
| Promotion plan taxonomy | Strong read-only governance layer |

## Internal Family Matrix

| Family | Current reading |
|--------|-----------------|
| Channels | Most mature internal SDK family |
| Tools | Strong internal catalog seam |
| Memory systems | Clear registry seam |
| Providers | Important convergence target, less unified than channels |

## Native extension authoring matrix

| Language | Current posture | Scaffolded runtime files | Governed smoke command | Checked-in example |
|----------|------------------|--------------------------|------------------------|--------------------|
| Python | Supported public runnable template | `index.py` | `loong plugins invoke-extension ... --allow-command python3` | `examples/plugins-process/native-extension-python/` |
| JavaScript | Supported public runnable template | `index.js` | `loong plugins invoke-extension ... --allow-command node` | `examples/plugins-process/native-extension-javascript/` |
| Go | Supported public runnable template | `main.go` | `loong plugins invoke-extension ... --allow-command go` | `examples/plugins-process/native-extension-go/` |
| Rust | Supported public runnable template | `Cargo.toml`, `src/main.rs` | `loong plugins invoke-extension ... --allow-command cargo` | `examples/plugins-process/native-extension-rust/` |
