# Plugin Manifest Format

Embedded manifest blocks can be stored inside comments of any source language.

## Markers

- start marker: `LOONGCLAW_PLUGIN_START`
- end marker: `LOONGCLAW_PLUGIN_END`

## JSON Fields

Required/commonly used fields:

- `plugin_id`
- `provider_id`
- `connector_name`
- `capabilities` (array of `Capability` enum values)

Optional fields:

- `channel_id`
- `endpoint`
- `metadata` (string map)
- `summary`
- `tags`
- `input_examples`
- `output_examples`
- `defer_loading`

## Minimal Example

```text
// LOONGCLAW_PLUGIN_START
// {
//   "plugin_id": "openrouter-rs",
//   "provider_id": "openrouter",
//   "connector_name": "openrouter",
//   "channel_id": "primary",
//   "endpoint": "https://openrouter.ai/api/v1/chat/completions",
//   "capabilities": ["InvokeConnector"],
//   "metadata": {"version": "0.5.0"}
// }
// LOONGCLAW_PLUGIN_END
```

## Related Docs

- [Plugin Runtime Governance](./plugin-runtime-governance.md)
- [Spec Runner Reference](./spec-runner.md)
