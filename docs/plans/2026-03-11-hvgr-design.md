# HVGR Design (Alpha-Test)

Date: 2026-03-11  
Scope: `loongclaw-ai/loongclaw` `alpha-test` branch

## 1. Problem Statement

Current conversation handling in `crates/app` is a single-turn tool-aware flow:

- provider returns one turn
- tools execute once (bounded by `max_tool_steps`)
- optional natural-language follow-up completion

This is simple and useful for MVP, but it does not yet provide a robust execution substrate for:

- high-complexity multi-step tasks
- strong replayability and deterministic validation
- lane-based performance/safety tradeoff under policy control

## 2. Design Goals

We optimize for both extremes simultaneously:

1. High performance:
   - low latency for easy tasks
   - bounded context growth
   - predictable resource ceilings
2. High stability and safety:
   - deterministic policy-guided routing
   - explicit validation before/after execution
   - fail-closed behavior under ambiguous risk
3. Long-term sustainability:
   - additive contracts
   - config-driven thresholds and behavior
   - testable architecture primitives

## 3. Target Architecture

### 3.1 Hybrid Lane Runtime

Use two execution lanes selected by deterministic policy:

- `fast` lane: low-risk and low-complexity requests, optimized for latency.
- `safe` lane: higher-risk or higher-complexity requests, optimized for control and bounded behavior.

Lane selection should be deterministic and explainable (`reason` list), not prompt-only.

### 3.2 Plan IR (Execution Graph Contract)

Represent execution intent in a bounded graph contract:

- explicit nodes and edges
- per-node retry/timeout/risk metadata
- global budget limits
- deterministic graph validation (`DAG`, refs, budget)

This enables future transition from single-turn execution to verified graph execution without API breakage.

### 3.3 Verify/Replan Path (Future Stage)

After graph execution, run explicit verifier checks:

- schema validity
- goal completion checks
- safety and policy postconditions

If verification fails, replan only failed subgraph, not full replay.

## 4. Alpha-Test Implementation (This Iteration)

### 4.1 Added `lane_arbiter` primitive

File: `crates/app/src/conversation/lane_arbiter.rs`

Delivered:

- `ExecutionLane` (`fast`/`safe`)
- `LaneArbiterPolicy` with deterministic scoring:
  - risk score (keyword-based)
  - complexity score (token/connector/punctuation heuristics)
  - input-length threshold
- `LaneDecision` with explainable reasons

### 4.2 Added `plan_ir` primitive

File: `crates/app/src/conversation/plan_ir.rs`

Delivered:

- `PlanGraph`, `PlanNode`, `PlanEdge`, `PlanBudget`
- node-level risk tier + retry/timeout metadata
- strict validator:
  - non-empty graph
  - unique node ids
  - known edge references
  - no self-loop
  - at least one entry and one terminal node
  - attempt and node budget checks
  - `timeout_ms > 0` enforcement
- acyclic graph enforcement (Kahn-style topological validation)

### 4.2b Added `plan_executor` primitive

File: `crates/app/src/conversation/plan_executor.rs`

Delivered:

- deterministic topological execution over validated `PlanGraph`
- per-node retry loop (`max_attempts`)
- per-node timeout enforcement (`timeout_ms`)
- global attempt budget enforcement (`max_total_attempts`)
- wall-time budget enforcement (`max_wall_time_ms`)
- structured run report:
  - `PlanRunStatus` (`Succeeded` / `Failed`)
  - failure taxonomy (`ValidationFailed`, `BudgetExceeded`, `WallTimeExceeded`, `NodeFailed`)
  - ordered node trace + per-attempt execution events

### 4.3 Light integration in turn coordinator

Files:

- `crates/app/src/conversation/turn_coordinator.rs`
- `crates/app/src/conversation/orchestrator.rs` (compat alias)

Delivered:

- lane arbitration invoked before tool execution.
- lane result selects configurable tool-step limits:
  - `conversation.fast_lane_max_tool_steps_per_turn`
  - `conversation.safe_lane_max_tool_steps_per_turn`
- lane policy now uses config surface:
  - `conversation.hybrid_lane_enabled`
  - `conversation.safe_lane_risk_threshold`
  - `conversation.safe_lane_complexity_threshold`
  - `conversation.fast_lane_max_input_chars`
  - `conversation.high_risk_keywords`
  - optional safe-lane plan path (default off):
  - `conversation.safe_lane_plan_execution_enabled`
  - `conversation.safe_lane_node_max_attempts`
  - `conversation.safe_lane_plan_max_wall_time_ms`
  - `conversation.safe_lane_verify_output_non_empty`
  - `conversation.safe_lane_verify_min_output_chars`
  - `conversation.safe_lane_verify_require_status_prefix`
  - `conversation.safe_lane_verify_adaptive_anchor_escalation`
  - `conversation.safe_lane_verify_anchor_escalation_after_failures`
  - `conversation.safe_lane_verify_anchor_escalation_min_matches`
    - `conversation.safe_lane_emit_runtime_events`
    - `conversation.safe_lane_event_sample_every`
  - `conversation.safe_lane_verify_deny_markers`
  - `conversation.safe_lane_replan_max_rounds`
  - `conversation.safe_lane_replan_max_node_attempts`
  - `conversation.safe_lane_backpressure_guard_enabled`
  - `conversation.safe_lane_backpressure_max_total_attempts`
  - `conversation.safe_lane_backpressure_max_replans`
  - `conversation.safe_lane_session_governor_enabled`
  - `conversation.safe_lane_session_governor_window_turns`
  - `conversation.safe_lane_session_governor_failed_final_status_threshold`
  - `conversation.safe_lane_session_governor_backpressure_failure_threshold`
  - `conversation.safe_lane_session_governor_trend_enabled`
  - `conversation.safe_lane_session_governor_trend_min_samples`
  - `conversation.safe_lane_session_governor_trend_ewma_alpha`
  - `conversation.safe_lane_session_governor_trend_failure_ewma_threshold`
  - `conversation.safe_lane_session_governor_trend_backpressure_ewma_threshold`
  - `conversation.safe_lane_session_governor_recovery_success_streak`
  - `conversation.safe_lane_session_governor_recovery_max_failure_ewma`
  - `conversation.safe_lane_session_governor_recovery_max_backpressure_ewma`
  - `conversation.safe_lane_session_governor_force_no_replan`
  - `conversation.safe_lane_session_governor_force_node_max_attempts`
  - when enabled, safe lane runs a bounded `PlanGraph` via `PlanExecutor` instead of direct `TurnEngine` multi-intent execution.
  - bounded replan policy:
    - retries only for retryable runtime node failures (for example transient tool execution failures)
    - does not replan on policy denials / topology / static validation failures
    - retryability now comes from typed node error kind (`retryable`, `policy_denied`, `non_retryable`), not string heuristics
    - subgraph replay optimization:
      - when a specific tool node fails, replan restarts from that failed tool node (preserves successful prefix outputs)
      - avoids replaying already-successful upstream tool nodes
  - structured runtime events for safe lane plan path:
    - `lane_selected`
    - `plan_round_started`
    - `plan_round_completed`
    - `verify_failed`
    - `verify_policy_adjusted`
    - `replan_triggered`
    - `final_status`
  - adaptive verify policy (quality convergence guard):
    - verification anchor requirement can auto-escalate after repeated verify failures
    - escalation is deterministic and config-driven, and emits `verify_policy_adjusted` runtime events
    - objective: reduce low-signal retry loops by tightening semantic output checks when instability persists
  - backpressure guard (performance + stability):
    - retryable failures are still eligible for replan, but only within bounded attempt/replan pressure budgets
    - when pressure guard is exceeded, route is forced terminal with explicit reason (`backpressure_attempts_exhausted` / `backpressure_replans_exhausted`)
    - objective: prevent retry storms under degraded tool/runtime conditions
  - session governor (multi-turn stability guard):
    - safe lane now reads recent session runtime history (best-effort) and summarizes prior safe-lane outcomes
    - governor history window is decoupled from chat `sliding_window` cap via an explicit extended-limit read path, so long-window governance does not silently collapse to short chat context defaults
    - governor engages when historical pressure exceeds configured thresholds:
      - failed `final_status` count threshold
      - backpressure-related failure-code count threshold
      - EWMA trend threshold on failed-final-status and backpressure-failure signals
    - governor supports recovery suppression to reduce oscillation:
      - requires trailing success streak and low EWMA pressure
      - when recovery threshold is met, governor can stay disengaged even if static count thresholds were previously crossed
    - once engaged, runtime policy is tightened for the current turn:
      - optional force-no-replan mode (`effective_max_rounds=0`)
      - clamp effective node retry ceiling (`effective_max_node_attempts`)
    - when no-replan is governor-driven, route reason is explicit (`session_governor_no_replan`) with dedicated terminal failure codes, avoiding ambiguity with ordinary round-budget exhaustion
    - governor decision (including trend/recovery diagnostics) is embedded into `lane_selected` and `plan_round_started` event payloads for auditability
  - runtime event sampling/throttling:
    - non-critical round-level events are sampled by `safe_lane_event_sample_every` (default `1` = no sampling)
    - critical events (`lane_selected`, `verify_failed`, `final_status`) are always emitted (never sampled out)
    - adaptive failure-pressure override:
      - when enabled (`safe_lane_event_adaptive_sampling=true`), non-critical events with explicit failure signals can bypass normal round sampling
      - failure-pressure threshold is configurable via `safe_lane_event_adaptive_failure_threshold`
      - keeps low steady-state audit volume while increasing observability density during instability
  - event analytics interface:
    - `conversation::analytics::parse_conversation_event`
    - `conversation::analytics::summarize_safe_lane_events`
    - produces `SafeLaneEventSummary` with:
      - terminal status, failure code, route decision
      - terminal route reason + route-reason distribution
      - latest metrics snapshot + snapshot count
      - governor trigger counters + latest governor trend/recovery snapshot
      - rollup distributions (`route_decision_counts`, `route_reason_counts`, `failure_code_counts`, `final_status_counts`)
      - verify-policy adjustment event count (`verify_policy_adjusted_events`)
    - parser is backward-compatible with sparse/partially sampled payloads (missing metrics fields default to zero)
  - analytics consumer path (runtime diagnostics):
    - interactive CLI now supports `/safe_lane_summary [limit]`
    - command reads persisted conversation events from session memory window and prints:
      - event counters
      - terminal status/failure/route
      - derived rates (`replan_per_round`, `verify_fail_per_round`)
      - latest metrics snapshot
      - route/failure rollups
  - runtime audit promotion (kernel plane stream):
    - when `conversation.safe_lane_emit_runtime_events=true` and kernel context exists, each safe-lane runtime event is also emitted as `AuditEventKind::PlaneInvoked`
    - plane: `Runtime`, tier: `Core`
    - adapter id: `conversation.safe_lane`
    - operation format: `conversation.safe_lane.<event_name>`
    - this keeps event persistence in conversation history while adding first-class kernel-observable execution telemetry
  - history hygiene guard:
    - internal structured records (`conversation_event`, `tool_decision`, `tool_outcome`) are filtered from provider history window construction
    - unknown/non-chat roles in persisted history are filtered out before provider request assembly
    - prevents observability metadata from polluting subsequent model context in multi-turn runs
  - kernel memory read-path hardening:
    - conversation history window loading now routes through kernel memory core (`operation=window`) when `kernel_ctx` is present
    - capability gate (`MemoryRead`) and memory plane audit are now enforced for both write path (`append_turn`) and read path (`window`)
    - closes the prior asymmetry where only persistence used kernel while history loading used direct memory path
  - verifier uplift:
    - expected result lines are now derived from planned tool intent count (instead of output self-counting)
    - status-level failure detection (`[error]`, `[denied]`, `[failed]`, etc.)
    - optional semantic anchor matching from tool intent args metadata (observability signal by default, not hard gate)
    - verify failures now use retryability classification:
      - retryable verify failures can trigger bounded replan
      - non-retryable verify failures fail fast without wasting replan rounds
- naming aligned with actual responsibility:
  - canonical type: `ConversationTurnCoordinator`
  - `ConversationOrchestrator` retained as backward-compatible alias
- failure taxonomy unification across fast/safe execution surfaces:
  - `TurnResult` failure variants now carry structured `TurnFailure` metadata
  - shared fields: `kind`, `code`, `reason`, `retryable`
  - `TurnFailureKind` values:
    - `approval_required`
    - `policy_denied`
    - `retryable`
    - `non_retryable`
    - `provider`
  - fast-lane (`TurnEngine`) and safe-lane (`turn_result_from_plan_failure`) now map into the same taxonomy envelope, while preserving existing inline message compatibility via `reason`

This keeps current behavior stable while introducing migration hooks.

## 5. Validation Strategy

### 5.1 Unit tests for new primitives

- lane arbitration:
  - simple low-risk request -> `fast`
  - high-risk keyword request -> `safe`
  - complex multi-clause request -> `safe`
- plan IR:
  - valid DAG pass
  - duplicate node id fail
  - unknown edge ref fail
  - cycle fail
  - budget overflow fail
- plan verifier:
  - rejects failure status lines
  - supports semantic anchor mismatch detection
- safe lane runtime events:
  - event persistence enabled/disabled behavior is covered by regression tests
  - kernel runtime-plane audit emission enabled/disabled behavior is covered by regression tests
- kernel-routed memory history loading:
  - `build_messages` kernel path issues `memory.window` request with session + sliding-window limit
  - regression test verifies request routing and memory-plane audit visibility
- unified failure taxonomy:
  - unknown-tool validation path exposes `policy_denied/tool_not_found`
  - transient kernel tool execution error path exposes `retryable/tool_execution_failed`
  - kernel-error classification is now shared (single mapping table) across fast lane (`TurnEngine`) and safe lane (`PlanNodeError` translation), preventing taxonomy drift
  - safe-lane runtime events (`verify_failed`, `plan_round_completed[failed]`, `final_status[failed]`) now carry:
    - `failure_kind`
    - `failure_code`
    - `failure_retryable`
    - `route_decision`
    - `route_reason`
  - safe-lane now emits structured execution metrics snapshot in runtime event payloads (`final_status` and round events):
    - `rounds_started`
    - `rounds_succeeded`
    - `rounds_failed`
    - `verify_failures`
    - `replans_triggered`
    - `total_attempts_used`
  - safe-lane failure routing table is explicitly validated by unit tests:
    - `policy_denied` => terminal
    - `retryable` + remaining rounds => replan
    - `retryable` + exhausted rounds => terminal
    - `non_retryable/provider/approval_required` => terminal

### 5.2 Regression expectation

Existing conversation tests should continue to pass because:

- no breaking contract changes to `TurnEngine`
- no mandatory config migration yet
- coordinator output behavior remains compatible

## 6. Next Iteration Plan

1. Add production-grade metrics export/aggregation from runtime events:
   - lane distribution
   - replan rate
   - verify-failure distribution
   - budget exhaustion rate
   - currently available in CLI summary; next step is daemon/API export path
2. Evaluate selective event sampling/throttling for high-frequency runtime events to control audit volume under heavy multi-turn load.
3. Add adaptive verify-policy tuning:
   - when repeated retryable verify failures occur, tighten/adjust verification context before next replan
   - keep bounded behavior and explicit audit trail for every policy adjustment

## 7. Risks and Mitigations

1. Heuristic misrouting risk:
   - mitigation: configurable thresholds + audited decision reasons
2. Overfitting risk from static keywords:
   - mitigation: additive policy tuning and test corpus updates
3. Partial architecture drift:
   - mitigation: keep Plan IR validator mandatory in new execution path

## 8. Acceptance Criteria (Alpha-Test)

1. New primitives compile and test pass.
2. Existing conversation behavior remains stable.
3. Design contract is documented in repository and reviewable.
