# Programmatic Tool Call

`programmatic_tool_call` executes deterministic multi-step connector orchestration inside one spec run.

## Step Kinds

- `set_literal`
- `json_pointer`
- `connector_call`
- `connector_batch`
- `conditional`

## Core Guarantees

- max-call budgeting (`max_calls`)
- connector allowlist enforcement (`allowed_connectors`)
- automatic caller provenance injection (`_loongclaw.caller`)
- payload templating (`{{step_id}}`, `{{step_id#/json/pointer}}`)
- typed programmatic error format (`programmatic_error[<code>]`)

## Retry and Throughput Controls

Per call (`connector_call` and batch calls):

- retry fields: `max_attempts`, `initial_backoff_ms`, `max_backoff_ms`
- deterministic adaptive jitter: `jitter_ratio`, `adaptive_jitter`

Per connector (operation-level policy):

- rate shaping: `connector_rate_limits.<connector>.min_interval_ms`
- circuit breaker: `failure_threshold`, `cooldown_ms`,
  `half_open_max_calls`, `success_threshold`

## Adaptive Concurrency Policy

`concurrency` config controls parallel fanout behavior:

- `max_in_flight`: upper bound of concurrent calls
- `min_in_flight`: lower bound during adaptive contraction
- `fairness`: `weighted_round_robin` or `strict_round_robin`
- priority weights: `high_weight`, `normal_weight`, `low_weight`
- adaptive behavior:
  - `adaptive_budget`
  - `adaptive_recovery_successes`
  - `adaptive_upshift_step`
  - `adaptive_downshift_step`
  - `adaptive_reduce_on` trigger set

Each `connector_call` and each `connector_batch.calls[*]` supports `priority_class`:

- `high`
- `normal`
- `low`

## Batch Execution Output

`connector_batch` output includes:

- `calls` (ordered report array)
- `by_call` (lookup map)
- scheduler telemetry:
  - `dispatch_order`
  - `peak_in_flight`
  - `configured_max_in_flight`
  - `configured_min_in_flight`
  - `budget_reductions`
  - `budget_increases`
  - `final_in_flight_budget`
  - adaptive strategy fields (`adaptive_*`, `adaptive_reduce_on`)

Per-call error report includes `error_code` for deterministic remediation routing:

- `connector_not_found`
- `connector_not_allowed`
- `capability_denied`
- `policy_denied`
- `circuit_open`
- `connector_execution_error`

## Suggested Verification Matrix

- budget enforcement (`max_calls`) and duplicate call-id rejection
- retry/jitter accumulation and deterministic replay behavior
- rate shaping wait behavior under repeated same-connector calls
- circuit open blocking and half-open recovery transition
- fairness/non-starvation across mixed priority classes
- adaptive budget reduction triggers and floor/ceiling behavior

## Related Docs

- Example spec: [programmatic-tool-call.json](../../examples/spec/programmatic-tool-call.json)
- Spec runner: [Spec Runner Reference](./spec-runner.md)
