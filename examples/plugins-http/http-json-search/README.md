# HTTP JSON Search Example

This example shows the checked-in generic endpoint-backed connector package lane.

## Scaffold

```bash
loong plugins init ./search-http \
  --plugin-id search-http \
  --bridge-kind http_json \
  --endpoint https://localhost/invoke \
  --connector-operation search
```

## Validate

```bash
loong plugins doctor --root "examples/plugins-http/http-json-search" --profile sdk-release
```

## Inspect

```bash
loong plugins inventory --root "examples/plugins-http/http-json-search"
```

## Probe

```bash
loong plugins invoke-connector-operation \
  --root "examples/plugins-http/http-json-search" \
  --plugin-id http-json-search-example \
  --operation search \
  --payload '{}'
```

## What it proves

- `bridge_kind=http_json`
- `entrypoint=https://localhost/invoke`
- `loong_connector_operations_json`
- `loong_connector_operation_specs_json`
