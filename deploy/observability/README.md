# Agent Observability

Loong is instrumented with OpenTelemetry for trace analysis. This directory provides an example backend using Jaeger and the OpenTelemetry Collector.

## Quick Start

```bash
cd deploy/observability
docker compose up -d
```

## Endpoints

| Service | Endpoint | Description |
|---------|----------|-------------|
| OTel Collector OTLP HTTP | `http://localhost:4318` | Receive traces from Loong |
| Jaeger UI | `http://localhost:16686` | Visualize traces |

## Integration with Loong

Before running Loong, export the following environment variables:

```bash
export LOONG_OTEL_CAPTURE_CONTENT=1
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
loong ...
```
