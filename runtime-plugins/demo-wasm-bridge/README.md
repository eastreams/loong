# Demo WASM Bridge Plugin

This directory is a small, source-visible runtime plugin for customer demos.

- `loong.plugin.json` declares a `wasm_component` channel bridge.
- `plugin.wat` is the WebAssembly Text source loaded by the demo runtime.

The demo uses Loong's `loong_json_abi_v1` core WebAssembly ABI:

1. Host serializes the bridge request JSON.
2. Host calls guest `loong_alloc(len)` and writes the request bytes into exported `memory`.
3. Host calls guest `loong_invoke(ptr, len) -> i64`.
4. Guest returns `response_ptr << 32 | response_len`.
5. Host reads response JSON from guest memory and uses `payload` as the bridge response payload.

The current demo guest returns a fixed JSON payload with `via = "wasm"` and an empty `messages` array. It proves plugin discovery, activation, Wasmtime loading, and JSON request exchange without requiring external channel services.

## TUI-first smoke check

For a customer demo, the shortest path only needs a working Loong TUI provider API key. Start the TUI from the repository root:

```bash
cargo run -p loong
```

Then ask the agent to use the `plugin` tool to call the bundled demo plugin with a payload such as:

```json
{
  "payload": {
    "text": "hello from the TUI"
  }
}
```

Expected result:

- the model emits a `plugin` tool call
- the runtime selects `demo-wasm-bridge`
- the result payload contains `via = "wasm"`

See `site/build-on-loong/wasm-plugins.mdx` for the full usage guide.
