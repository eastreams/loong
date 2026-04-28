# Native Extension Go Example

This example shows the current public manifest-first Go native extension lane.

## Validate

```bash
loong plugins doctor --root "examples/plugins-process/native-extension-go" --profile sdk-release
```

## Inspect

```bash
loong plugins inventory --root "examples/plugins-process/native-extension-go"
```

## Smoke-test

```bash
loong plugins invoke-extension \
  --root "examples/plugins-process/native-extension-go" \
  --plugin-id native-extension-go-example \
  --method extension/event \
  --payload '{"event":"session_start"}' \
  --allow-command go
```
