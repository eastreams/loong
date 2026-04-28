# Native Extension TypeScript Example

This example shows the current public manifest-first TypeScript native extension lane.

## Validate

```bash
loong plugins doctor --root "examples/plugins-process/native-extension-typescript" --profile sdk-release
```

## Inspect

```bash
loong plugins inventory --root "examples/plugins-process/native-extension-typescript"
```

## Smoke-test

```bash
loong plugins invoke-extension \
  --root "examples/plugins-process/native-extension-typescript" \
  --plugin-id native-extension-typescript-example \
  --method extension/event \
  --payload '{"event":"session_start"}' \
  --allow-command node
```
