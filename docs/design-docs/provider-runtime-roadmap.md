# Provider Runtime Deep Assessment and Roadmap

## Scope and Baseline

- OpenClaw reference: `openclaw/openclaw` `main@936607c` (fetched on 2026-03-11).
- LoongClaw baseline: `loongclaw-ai/loongclaw` `alpha-test@cd39c7a`.
- Current working track: `feat/provider-runtime-abstraction` (this branch, unmerged changes).

This document captures architecture comparison, implemented hardening, and a durable roadmap for provider runtime evolution.

## Related Runtime Contracts

- Discovery-first tool routing contract:
  [`discovery-first-tool-runtime-contract.md`](discovery-first-tool-runtime-contract.md)
  fixes the provider-core tool surface, lease expectations, compatibility
  rewrites, and follow-up provider-turn behavior for the discovery-first tool
  path.

## Comparative Architecture Snapshot

| Dimension | OpenClaw (`main`) | LoongClaw (`alpha-test`) | Current branch (`feat/provider-runtime-abstraction`) |
| --- | --- | --- | --- |
| Provider capability abstraction | Dedicated capability matrix (`provider-capabilities`) and provider-family toggles (OpenAI/Anthropic/default) | Minimal kind-based branching in runtime path | Still lightweight kind-based flow, but transport mode abstraction and payload adaptation are centralized |
| Model catalog runtime | Shared async catalog loader with cache poisoning protection and synthetic forward-compat entries | Per-request direct model-list fetch in auto mode | Added bounded model-catalog cache + stale-if-error fallback + singleflight dedupe |
| Retry semantics | Multi-layer failover with provider/profile cooling and categorized failure reasons | Status/transport retry with exponential backoff only | Retry planner extracted; honors `Retry-After`, and central status/transport planning reused across paths |
| Fallback semantics | Auth-profile rotation first, model fallback second, explicit cooldown policy | Model fallback only when error text suggests mismatch | Two-stage baseline now active (profile-aware attempts + model fallback) with configurable profile cooldown/auth-disable policy |
| Auth routing | Auth profile store (`auth-profiles`), ordering and sticky selection | API key / oauth token env resolution | Runtime profile state now supports process restart persistence via local snapshot file; still lacks explicit operator-managed profile registry |
| Observability | Structured fallback decision logs and reason taxonomy | String-level error surfaces, limited reason normalization | Kernel audit events + runtime failover metrics baseline; still lacks persistent/exported telemetry sink |
| Config governance | Rich providers config normalization and runtime discovery pipeline | Single provider config with profile defaults and aliases | Added cache/profile tuning knobs with clamp boundaries and tests |

## Completed Hardening in This Branch

1. Provider request session abstraction:
- Built `ProviderRequestSession` to avoid duplicated endpoint/header/client/model preparation.
- Unified completion/turn dispatch by `request_across_model_candidates(...)`.

2. Model-catalog resilience:
- Added configurable cache TTL and stale fallback windows:
  - `model_catalog_cache_ttl_ms`
  - `model_catalog_stale_if_error_ms`
  - `model_catalog_cache_max_entries`
- Added bounded memory behavior and dead-entry pruning.
- Added singleflight dedupe for concurrent model-list fetches with panic/leader-drop recovery guard.
- Added stale singleflight leader takeover to prevent follower starvation when a leader stalls abnormally.
- Hardened cache-key derivation to avoid raw secret leakage.

3. Retry planner and policy consolidation:
- Moved retry decisions into planner helpers for status and transport failures.
- Added `Retry-After` support for both numeric seconds and HTTP-date formats.
- Preserved bounded backoff behavior with minimum/maximum constraints.

4. Safer model-switch policy:
- Auto-switch to next model is now gated by model-semantic status classes (`400/404/410/422`) plus semantic error evidence.
- Prevents switching on exhausted `5xx` scenarios where retry/failure is safer than model churn.

5. Test coverage expansion:
- Added cache behavior, singleflight concurrency, panic recovery, cache-key secrecy tests.
- Added retry-delay policy tests, including HTTP-date parsing and stale-date handling.
- Added model request status planner tests for retry/switch/fail branching.

6. Structured failure reason baseline:
- Added internal `ProviderFailoverReason` classification across status/transport/decode/shape failures.
- Status failures now normalize into typed reasons (`rate_limited`, `provider_overloaded`, `auth_rejected`, `payload_incompatible`, etc.) to support future telemetry and profile-level routing.
- Provider error boundaries now embed a parseable `provider_failover={...}` JSON snapshot (reason/stage/model/attempt/status), enabling downstream log pipelines to consume machine-friendly diagnostics without brittle text parsing.

7. Kernel-audited failover event baseline:
- Added first-class `AuditEventKind::ProviderFailover` events when kernel context is available.
- Events persist provider/runtime failover dimensions (`provider_id`, `reason`, `stage`, `model`, `attempt`, `status_code`, candidate index/count) for postmortem and analytics pipelines.

8. Runtime contract baseline:
- Added `ProviderRuntimeContract` as a single abstraction point for provider default token field and transport mode.
- Replaced scattered provider-kind defaults with contract-driven resolution to make future provider onboarding deterministic.

9. Cross-request model candidate cooldown:
- Added bounded model-candidate cooldown state for auto model mode.
- Models that fail with durable incompatibility reasons (for example model mismatch) are deprioritized with bounded exponential backoff and a configurable cap, reducing repeated failed first attempts while avoiding unbounded suppression.

10. Unified failover observability baseline:
- Provider failover events now emit through a unified runtime event path that records process-local metrics regardless of kernel context.
- Added failover KPI-ready counters (`total_events`, `continued_events`, `exhausted_events`) and dimension aggregations by `provider`, `reason`, and `stage`.
- Kept kernel audit emission as a best-effort side path, so non-kernel runtimes no longer lose typed failover signals.

11. Provider profile health + two-stage fallback baseline:
- Added runtime `ProviderProfileStateStore` with profile-level cooldown/disable windows and `last_used` tracking.
- Added auth profile resolution for provider credentials (`oauth` and `api_key`) with profile health ordering.
- Request path now executes profile-aware failover before model fallback for each model candidate, and model catalog fetching can rotate across profiles.
- This is currently process-local (in-memory) state and acts as the migration baseline toward persistent profile health.

12. Multi-key profile expansion baseline:
- `ProviderConfig` now supports parsing delimited API key pools (comma/semicolon/newline) and `*_API_KEYS` style envs via candidate expansion.
- Provider runtime can materialize multiple API-key-backed auth profiles from one config/env source, enabling real profile rotation instead of single-key fallback.

13. Profile policy parameterization baseline:
- Profile-health runtime controls are now configurable and bounded through provider config:
  - `profile_cooldown_ms`
  - `profile_cooldown_max_ms`
  - `profile_auth_reject_disable_ms`
  - `profile_state_max_entries`
- Runtime policy construction now consumes resolved config values instead of hardcoded constants, with clamp/default regression tests.

14. Persistent profile-health snapshot baseline:
- Added local durable snapshot for provider profile state (`~/.loongclaw/provider-profile-state.json`) with best-effort atomic write (tmp + rename fallback).
- Runtime now reloads profile cooldown/disable health state on process startup and preserves in-flight health signals across restarts.
- Added roundtrip/unknown-reason snapshot tests to guard migration and forward-compat behavior.
- Snapshot persistence now uses lock-outside file I/O (capture state under lock, persist after lock release) to reduce request-path mutex contention under concurrent traffic.
- Snapshot persistence now carries a monotonic `revision` with serialized file-write gate to prevent stale snapshot overwrite during concurrent updates.
- Snapshot entry ordering is now deterministic by key for stable artifacts and lower diff/noise during diagnostics.

15. Pluggable persistence backend baseline:
- Introduced `ProviderProfileStateBackend` abstraction with default file backend (runtime) and in-memory backend (tests).
- Profile-state load/persist wiring is now backend-driven instead of hardcoded global file helpers, enabling incremental migration toward sqlite/redis without touching request-path logic.

16. Sqlite profile-state backend (optional) baseline:
- Added `provider.profile_state_backend` selection (`file` default, `sqlite` optional) and `provider.profile_state_sqlite_path` override.
- Sqlite backend persists snapshots in `provider_profile_state` with revision-aware upsert (`ON CONFLICT ... WHERE excluded.revision >= current`) to guard stale cross-process writes.
- Backend initialization keeps backward compatibility by preserving file backend as default when no override is configured.
- Sqlite cold-start can import legacy file snapshot (`provider-profile-state.json`) when database row is empty, preserving profile health continuity during backend migration.

17. Profile-state persistence observability baseline:
- Added persistence outcome counters for `persisted`, `stale_skipped`, and `failed` writes.
- Persistence metrics are now updated by backend outcomes (file/sqlite), enabling future runtime export hooks without changing request-path logic.
- Added backend-level regression tests for stale revision guard behavior (file + sqlite paths).

18. Contractized provider gate/validation/payload paths:
- `ProviderRuntimeContract` now carries explicit feature-family mapping and validation contract fields.
- Runtime feature gating (`provider-openai` / `provider-volcengine`), provider configuration validation (Kimi coding endpoint and KimiCLI user-agent constraints), and kimi `extra_body` payload injection are all contract-driven instead of scattered provider-kind branches.
- Added regression coverage for contract defaults and non-kimi extra-body omission to reduce future provider onboarding regression risk.

19. Contractized profile-health enforcement mode:
- Added profile-health mode to runtime contracts with explicit `enforce` vs `observe_only` semantics.
- `openrouter` now uses observe-only profile-health routing: failure counters remain observable, but cooldown/disable windows do not hard-suppress profile selection.
- Profile-state policy construction now derives health mode from runtime contract, avoiding scattered provider-specific routing exceptions.
- Added regression tests for policy mode resolution and observe-only cooldown bypass behavior.

20. Configurable profile-health mode override:
- Added `provider.profile_health_mode` config (`provider_default` / `enforce` / `observe_only`) so operators can override provider-family defaults without patching runtime code.
- Runtime contract resolution now first applies explicit config override, then falls back to provider defaults.
- This keeps long-term flexibility for production policy tuning while preserving deterministic defaults for compatibility.

21. Payload adaptation progression contractized:
- `ProviderRuntimeContract` now carries an explicit payload adaptation progression contract (token/reasoning/temperature fallback sequence), instead of relying on per-parameter branching spread in retry loops.
- Runtime adaptation now uses monotonic progression (`default -> alternate -> omit`) per axis, preventing retry cycles when providers reject both primary and fallback parameter names.
- Added regression tests for contract progression defaults and no-cycle behavior across token/reasoning fallback chains.

22. Payload incompatibility classification aligned to runtime contract:
- Status-failure reason classification now consults runtime contract payload-adaptation descriptors instead of global heuristic checks.
- Error-shape matching is centralized in contract descriptors (token/reasoning/temperature parameter sets + temperature-default phrase hints), reducing drift between adaptation and failover reasoning paths.
- Request-path and status-classification paths now share one contract surface for payload incompatibility semantics.

23. Provider contract/adaptation module decomposition baseline:
- Extracted runtime contract + payload adaptation + provider error-shape parsing into `crates/app/src/provider/contracts.rs`.
- `provider/mod.rs` now consumes the contracts module as a stable boundary, reducing monolithic file pressure and making future provider-family extension less conflict-prone.
- This establishes the first practical decomposition seam for continued split into request planning and profile-state backend layers.

24. Request planner module decomposition baseline:
- Extracted status/transport retry planning and model-switch/failure-reason classification into `crates/app/src/provider/request_planner.rs`.
- Both completion and turn request paths now consume one shared planner surface (`plan_model_request_status`, `plan_status_retry`, `plan_transport_error_retry`), reducing duplicated retry-branch drift.
- Payload incompatibility and model-switch semantics now flow through a single planner entrypoint, improving future maintainability for policy tuning.

25. Failover domain module decomposition baseline:
- Extracted failover domain entities (`ProviderFailoverReason`, `ProviderFailoverStage`, `ProviderFailoverSnapshot`, `ModelRequestError`) and error-envelope construction into `crates/app/src/provider/failover.rs`.
- `provider/mod.rs` now focuses on orchestration flow while failover semantics and snapshot serialization are isolated as a reusable boundary.
- This establishes a stable seam for future typed-event export and cross-module failover policy evolution without re-expanding the monolithic runtime file.

26. Unified request executor decomposition baseline:
- Extracted shared model-request state machine (status/transport retry, payload adaptation progression, model switch plan, failover error envelope) into `crates/app/src/provider/request_executor.rs`.
- `request_completion_with_model` and `request_turn_with_model` now only define strategy differences (request body builder, success parser, tool-schema downgrade hook), while execution semantics stay centralized.
- This removes duplicated control-flow branches across completion/turn paths and lowers long-term drift risk when tuning retry/failover policy.

27. Model-catalog executor decomposition baseline:
- Extracted model-list request loop (status/transport retry, response decode, empty-catalog guard) into `crates/app/src/provider/catalog_executor.rs`.
- `resolve_request_models` and provider profile-aware catalog fetch paths now consume `ModelCatalogRequestRuntime`, aligning catalog fetch flow with request executor style boundaries.
- This removes catalog-request retry state machine duplication from `provider/mod.rs` and reduces future drift between model-catalog resilience policy and main request-path resilience policy.

28. Profile-health policy decomposition baseline:
- Extracted profile-health decision policy (failure-message classification, failure-mark gating, health-based profile ordering) into `crates/app/src/provider/profile_health_policy.rs`.
- `provider/mod.rs` now keeps backend/state persistence orchestration while delegating pure policy decisions to the new module.
- Added dedicated policy-level tests for classification and health-order semantics, reducing long-term drift risk between enforcement and observe-only modes.

29. Profile-state backend decomposition baseline:
- Extracted provider profile-state persistence backend orchestration into `crates/app/src/provider/profile_state_backend.rs` (backend trait, file/sqlite implementations, revision gate, persistence metrics, backend bootstrap and state-store loader entrypoints).
- `provider/mod.rs` now consumes `ensure_provider_profile_state_backend(...)`, `with_provider_profile_states(...)`, and `persist_provider_profile_state_snapshot(...)` as orchestration calls instead of embedding backend internals.
- File/sqlite stale-revision guards and persistence metrics behavior remain covered by existing regression tests without behavior drift.

30. Profile-state store decomposition baseline:
- Extracted profile-state domain store and snapshot codec into `crates/app/src/provider/profile_state_store.rs` (`ProviderProfileStateStore`, `ProviderProfileStateSnapshot`, health mode/snapshot primitives, snapshot timestamp helpers).
- Profile-state domain serialization/versioning logic is now isolated from request/session orchestration, reducing `provider/mod.rs` monolith pressure and lowering long-term merge-conflict risk.
- Existing state roundtrip/revision/unknown-reason compatibility tests continue passing against the extracted module surface.

31. Model-catalog runtime decomposition baseline:
- Extracted model-catalog runtime state layer into `crates/app/src/provider/catalog_runtime.rs` (cache store, stale/fresh lookup semantics, singleflight slot coordination, stale-slot eviction recovery).
- `provider/mod.rs` now consumes catalog runtime APIs (`load_cached_model_catalog`, `store_model_catalog`, `fetch_model_catalog_singleflight`) rather than embedding cache/singleflight internals.
- Existing cache behavior and singleflight resilience tests (deduplicate, panic recovery, stale leader recovery) continue passing against the module boundary, preserving behavior while reducing orchestrator file pressure.

32. Model-candidate cooldown runtime decomposition baseline:
- Extracted model-candidate cooldown runtime state and ordering logic into `crates/app/src/provider/model_candidate_cooldown_runtime.rs` (policy object, cooldown cache/backoff state, failure registration gate, candidate prioritization strategy).
- `provider/mod.rs` now retains only cooldown policy construction and delegates runtime mutation/ordering behavior to the cooldown module surface.
- Existing cooldown behavior tests (reorder, non-replacement-reason ignore, exponential cap, expiry reset) continue passing after migration, preserving failure-routing semantics while reducing `provider/mod.rs` decision-state coupling.

33. Provider keyspace decomposition baseline:
- Extracted provider keyspace/hash derivation helpers into `crates/app/src/provider/provider_keyspace.rs` (`build_provider_cache_key`, model-catalog key, model-candidate cooldown namespace, profile-state namespace/key, auth-profile id key).
- `provider/mod.rs` now consumes centralized keyspace helpers instead of embedding duplicate hash routines, reducing cross-feature drift risk between catalog/cache/profile/auth key derivation.
- Existing secrecy and key stability tests continue passing after extraction, preserving cache/profile identity semantics while tightening keyspace ownership boundaries.

34. Failover telemetry runtime decomposition baseline:
- Extracted failover telemetry runtime internals into `crates/app/src/provider/failover_telemetry_runtime.rs` (event shaping, process-local metrics aggregation, kernel audit emission bridge).
- `provider/mod.rs` now delegates failover event recording to the telemetry runtime module, keeping request orchestration focused on retry/fallback flow control.
- Existing audit/metrics regression tests continue passing after migration, preserving provider failover observability semantics while reducing orchestrator coupling to telemetry state management.

35. Auth profile runtime decomposition baseline:
- Extracted auth-profile identity and credential expansion lifecycle into `crates/app/src/provider/auth_profile_runtime.rs` (`ProviderAuthProfile`, profile-id derivation bridge, credential dedup and anonymous fallback policy).
- `provider/mod.rs` now consumes `resolve_provider_auth_profiles(...)` from module boundary and no longer embeds auth-profile roster construction internals.
- Added module-level regression tests for bearer-header dedup and anonymous fallback behavior under env-isolated config, and retained existing provider-level auth profile ordering/expansion tests to guard orchestration behavior.

36. Profile-health runtime decomposition baseline:
- Extracted profile-health runtime orchestration into `crates/app/src/provider/profile_health_runtime.rs` (`ProviderProfileStatePolicy`, policy construction, profile success/failure state mutation, health-based auth profile ordering).
- `provider/mod.rs` now consumes profile-health runtime APIs and no longer embeds profile-state mutation or health-order flow details, further reducing orchestrator coupling.
- Existing policy/state regression tests continue passing after migration, preserving profile cooldown/disable semantics while tightening long-term maintainability boundaries between policy, runtime orchestration, and persistence backend layers.

37. Request-session runtime decomposition baseline:
- Extracted provider request-session planning flow into `crates/app/src/provider/request_session_runtime.rs` (`ProviderRequestSession`, session bootstrap orchestration, cooldown-policy derivation, auth-profile-aware model-candidate resolution path).
- `provider/mod.rs` now delegates session planning to `prepare_provider_request_session(...)`, further narrowing orchestrator responsibilities to request execution and response adaptation.
- Existing end-to-end provider runtime tests continue passing after migration, preserving auth-profile/model-candidate planning behavior while reducing monolithic session bootstrap coupling in `provider/mod.rs`.

38. Model-candidate resolver runtime decomposition baseline:
- Extracted request-model candidate resolution and ranking flow into `crates/app/src/provider/model_candidate_resolver_runtime.rs` (`resolve_request_models`, `rank_model_candidates`, and deduplicated candidate push helper).
- `request_session_runtime` now delegates request-model candidate discovery through resolver runtime module APIs, keeping session bootstrap orchestration separate from catalog cache/singleflight ranking internals.
- Existing model-catalog cache + ordering/cooldown regression tests continue passing after migration, preserving auto-model selection behavior while reducing candidate-resolution coupling in `provider/mod.rs`.

39. Request failover runtime decomposition baseline:
- Extracted cross-model/profile failover orchestration into `crates/app/src/provider/request_failover_runtime.rs` (`request_across_model_candidates`), including profile health mutation hooks, failover telemetry emission, and model-candidate cooldown registration.
- `provider/mod.rs` now keeps request entrypoint wiring and strategy-specific request execution hooks, while the multi-candidate failover state machine is isolated in a dedicated runtime module.
- Existing request-path, failover telemetry, and profile-health regression tests continue passing after migration, preserving two-stage fallback semantics while reducing long-term monolithic control-flow pressure in `provider/mod.rs`.

40. Catalog query runtime decomposition baseline:
- Extracted profile-aware model catalog query orchestration into `crates/app/src/provider/catalog_query_runtime.rs` (`fetch_available_models_with_profiles`), including auth-profile ordering and profile-health success/failure mutation flow.
- `provider/mod.rs` now delegates public `fetch_available_models(...)` to the catalog query runtime module, keeping orchestrator entrypoints thin and reducing direct coupling to profile/catalog sub-flows.
- Removed parent-module type-reexport coupling from `profile_state_store` by importing `ProviderProfileStatePolicy` from `profile_health_runtime` directly, hardening module-boundary independence for future decomposition.

41. HTTP client + payload runtime decomposition baseline:
- Extracted provider HTTP client construction into `crates/app/src/provider/http_client_runtime.rs` (`build_http_client`), and migrated catalog/session request flows to consume the module directly instead of `provider/mod.rs` local helpers.
- Extracted completion/turn payload assembly into `crates/app/src/provider/request_payload_runtime.rs` (`build_completion_request_body`, `build_turn_request_body`) with kimi extra-body shaping isolated inside payload runtime.
- `provider/mod.rs` now focuses on request entrypoint wiring and validation contracts, while transport client/runtime payload assembly are modularized as independent seams for future provider capability matrix evolution.

42. Provider validation runtime + dead path cleanup baseline:
- Extracted provider feature-gate and configuration validation into `crates/app/src/provider/provider_validation_runtime.rs` (`validate_provider_feature_gate`, `validate_provider_configuration`) and rewired request-session/catalog-query paths to consume this module directly.
- Removed duplicated validation helpers from `provider/mod.rs`, reducing orchestration-file policy drift risk and clarifying ownership of provider-family validation decisions.
- Removed legacy dead files `crates/app/src/provider/error_policy.rs` and `crates/app/src/provider/model_selection.rs` (unreferenced code paths) to avoid stale logic divergence and lower long-term maintenance overhead.

43. Request-dispatch runtime + validation-test ownership alignment baseline:
- Extracted model-bound request dispatch functions into `crates/app/src/provider/request_dispatch_runtime.rs` (`request_completion_with_model`, `request_turn_with_model`) and rewired `provider/mod.rs` to consume the module boundary directly.
- Continued slimming `provider/mod.rs` by removing in-file dispatch implementations and keeping orchestration entrypoints focused on session bootstrap + failover wiring.
- Relocated provider-configuration validation regression ownership into `provider_validation_runtime` module tests (including `kimi` coding-endpoint rejection and `kimi_coding` user-agent contract checks), consolidating behavior contracts with their implementation module.

44. Request-message runtime decomposition + filter contract hardening baseline:
- Extracted provider message assembly and session-window merge logic into `crates/app/src/provider/request_message_runtime.rs` (`build_system_message`, `build_base_messages`, `push_history_message`, `build_messages_for_session`) and rewired `provider/mod.rs` to thin wrappers.
- Removed message-filter implementation details from `provider/mod.rs`, reducing orchestration-module density and isolating prompt/history shaping policy in a dedicated runtime module.
- Added module-level regression tests for message filtering contracts (unsupported role drop, assistant internal event suppression, normal assistant replay pass-through) to harden long-term behavior stability while decomposition continues.

45. Error-classification contractization baseline:
- Introduced explicit `ProviderErrorClassificationContract` into `ProviderRuntimeContract` and moved model-switch/tool-schema unsupported heuristics behind contract fields (`model_not_found_codes`, `model_mismatch_message_fragments`, `tool_schema_error_parameters`, `tool_schema_error_message_fragments`).
- Rewired request planning and dispatch paths to consume runtime-contract-based classifiers (`plan_model_request_status`, `classify_model_status_failure_reason`, tool-schema downgrade hook), reducing reliance on scattered implicit string checks.
- Expanded runtime-contract regression assertions to lock contract defaults and avoid future provider-onboarding drift back into ad-hoc hardcoded error matching.

46. Payload-adaptation unsupported-parameter contract unification baseline:
- Removed remaining hardcoded unsupported-parameter message fragments from payload adaptation axis classification and switched to contract-owned fragments (`payload_adaptation.unsupported_parameter_message_fragments`).
- Introduced shared default fragment constants for error-classification + payload-adaptation contracts to keep error-shape semantics aligned across model-switch/tool-schema/payload adaptation decisions.
- Added regression assertions on payload adaptation contract defaults to prevent future drift where one error path updates fragment rules while another path silently lags behind.

47. Provider capability contract baseline:
- Introduced `ProviderCapabilityContract` into `ProviderRuntimeContract` to model runtime behavior capabilities explicitly (turn tool-schema default enablement, unsupported-tool-schema downgrade policy, reasoning extra-body support).
- Rewired turn dispatch and payload assembly to consume capability contract fields (`turn_tool_schema_enabled`, `tool_schema_downgrade_on_unsupported`, `include_reasoning_extra_body`) instead of ad-hoc inline booleans / provider-kind checks.
- Expanded runtime-contract regression assertions to lock capability defaults, establishing a stable contract seam for future provider-specific capability overrides.

48. Provider-scoped capability override matrix + classifier stress tests baseline:
- Upgraded capability resolution from family-only defaults to `provider kind` scoped overrides (`default + per-provider override`) in runtime contracts, aligning the abstraction pattern with OpenClaw's provider capability matrix trajectory.
- Kept behavior parity while moving reasoning extra-body enablement to provider-scoped override control (`kimi_coding`) so future provider quirks can be added without reopening request dispatch/payload logic.
- Added contract-level mixed-signal matrix tests and negative controls for tool-schema downgrade classification, model-switch classification, and payload-adaptation axis detection to harden long-term regression resistance.
- Removed legacy dead module `payload_adaptation.rs` (not in module graph) to eliminate stale heuristic drift risk and enforce single-source runtime contract ownership.

49. Capability strategy contract V2 baseline:
- Evolved provider capability contract from boolean toggles to strategy enums (`ProviderToolSchemaMode`, `ProviderReasoningExtraBodyMode`) with compatibility helper methods, so request paths consume policy intent rather than raw flags.
- Rewired request dispatch and payload assembly to use capability strategy helpers (`turn_tool_schema_enabled()`, `tool_schema_downgrade_on_unsupported()`, `include_reasoning_extra_body()`), preserving existing behavior while unlocking future mode expansion (`disabled`, `strict`, provider-specific schema policies).
- Expanded runtime regression assertions to validate strategy-level defaults for `openai` and `kimi_coding`, ensuring long-term capability changes remain contract-traceable instead of reintroducing ad-hoc branch logic.

50. Capability policy configuration override baseline:
- Added provider-level configuration controls for capability strategy policy (`provider.tool_schema_mode`, `provider.reasoning_extra_body_mode`) with explicit `provider_default` fallback semantics.
- Runtime contract assembly now applies config overrides after provider defaults, allowing operator-side policy tuning (for example `disabled` / `enabled_strict`) without touching request-runtime code.
- Added TOML parsing coverage and contract-level override regression tests so capability policy changes remain auditable, deterministic, and backward-compatible by default.

51. Model-hint capability override baseline:
- Added model-hint based capability override inputs in provider config (`tool_schema_disabled_model_hints`, `tool_schema_strict_model_hints`, `reasoning_extra_body_kimi_model_hints`, `reasoning_extra_body_omit_model_hints`) to support per-model behavior policy under one provider.
- Introduced model-aware capability resolver in runtime contracts (`resolve_provider_capability_for_model`) and rewired request dispatch/payload assembly to evaluate effective capability per target model.
- Added regression coverage for model-hint override matching and precedence (`disabled > strict`, `omit > kimi_thinking`) plus request-payload integration tests to harden long-term mixed-model compatibility behavior.

52. Model-aware capability resolution path optimization baseline:
- Refactored request dispatch to compute runtime-contract + model-effective capability once per model attempt and pass capability into payload builders, avoiding repeated contract/hint resolution inside each payload build/retry loop.
- Added capability-aware payload builder variants so runtime hot paths reuse pre-resolved capability decisions while preserving test-facing helper wrappers.
- Kept behavior parity under full regression suite while reducing repeated per-attempt policy derivation overhead in completion/turn execution flows.

53. Capability hint table extraction + planner capability semantics baseline:
- Added `ProviderCapabilityModelHints` as a normalized (trim/lowercase/dedup) capability-hint table so model-hint matching is precompiled from provider config instead of re-normalizing hint strings at each resolution call.
- `prepare_provider_request_session(...)` now precomputes runtime contract + capability hint table once and passes them through request dispatch/executor paths, reducing repeated contract/hint derivation across candidate attempts.
- Extended planner semantics with capability-aware status handling (`strict` tool-schema mode): when auto-model mode receives tool-schema unsupported errors for strict-capability models, planner now classifies as model mismatch and switches to next candidate instead of hard failing as generic payload incompatibility.
- Added regression coverage for hint-table normalization/reuse and strict tool-schema model-switch behavior to lock long-term determinism.

54. Capability profile runtime decomposition baseline:
- Extracted model-hint capability resolution into dedicated runtime module `capability_profile_runtime` (`ProviderCapabilityProfile`) so `contracts` remains focused on static provider contracts while runtime hint matching lives in one purpose-built boundary.
- Session bootstrap now materializes a reusable capability profile once per request session, and request dispatch/payload wrappers consume profile-based `resolve_for_model(...)` instead of contract-layer helper functions, reducing cross-module coupling and improving ownership clarity for future capability-matrix growth.
- Added dedicated module-level regression coverage for hint normalization/dedup, precedence stability (`disabled > strict`, `omit > kimi_thinking`), and base-capability fallback for non-matching models.

## Durability Gaps Still Present

1. Auth profile layer is missing:
- Profile roster supports multi-key expansion and now has local restart persistence, but still lacks a first-class operator-visible profile registry with explicit profile IDs/metadata lifecycle.
- Session-level profile stickiness and explicit per-profile operator controls are not yet implemented.
- Persistence now supports local file and sqlite, but remains single-node scoped (no shared/distributed profile state backend yet).

2. Structured failover event model is still incomplete:
- Runtime metrics now exist for both kernel and non-kernel paths, but there is no persistent/exported sink (for example JSONL/OTLP/prometheus bridge).
- SLO dashboards and alert thresholds are not yet wired to failover counters; observability is locally measurable but not yet operationalized.

3. Contract matrix for provider quirks is implicit:
- Payload adaptation progression is contractized, but error-shape detection still depends on provider text/parameter conventions.
- Next step is to move from error-text inference toward explicit per-provider capability descriptors loaded by contract.

4. End-to-end chaos validation is shallow:
- Unit tests are strong, but multi-provider integration and injected-failure scenarios are not yet fully systematized.

## Long-Term Sustainable Architecture Direction

### Phase A: Runtime Contracts (short horizon)

1. Introduce `ProviderCapabilityContract`:
- Explicit fields for token limits, reasoning schema mode, tool schema support, and endpoint compatibility.
- Replace string-only adaptation triggers with contract-first decisions.

2. Introduce `FailoverReason` enum at app runtime boundary:
- Normalize causes: `rate_limit`, `transport_timeout`, `auth_failed`, `model_not_found`, `unsupported_schema`, `server_overloaded`, `unknown`.
- Persist reason in internal events and logs.

### Phase B: Provider Health and Rotation (mid horizon)

1. Add `ProviderProfileStateStore`:
- Per profile/provider counters (`error_count`, `last_used_at`, `cooldown_until`, `disabled_until`).
- Cooldown strategy with bounded exponential policy.

2. Evolve local persistence to pluggable durable backend:
- Keep current JSON snapshot as compatibility path.
- Harden sqlite operations (migration/versioning/repair tooling), then add redis for multi-node deployments.
- Add migration/versioned schema strategy for profile state snapshots.
3. Two-stage failover engine:
- Stage 1: rotate profiles in provider.
- Stage 2: fallback to next model candidate.
- Optional stage 3: fallback across providers (when explicitly configured).

### Phase C: Observability and SLO Enforcement (long horizon)

1. Introduce structured runtime events:
- `provider_request_attempted`, `provider_retry_scheduled`, `provider_model_switched`, `provider_failover_exhausted`.

2. Define reliability SLOs:
- `p95_first_success_latency` by provider.
- `model_catalog_fetch_error_rate`.
- `fallback_exhaustion_rate`.

3. Add chaos test suite:
- Inject synthetic 429/503/auth failure/invalid schema patterns.
- Assert deterministic planner behavior and bounded retries.

## Verification Standard for Future Provider Changes

Every provider-runtime behavior change should pass all of the following:

1. Contract tests:
- Planner outputs are deterministic for status/error/capability matrices.

2. Concurrency tests:
- Singleflight and cache state remain safe under concurrent request pressure.

3. Regression tests:
- Existing compatible providers keep current default behavior.

4. Integration tests:
- At least one real provider mock path validates end-to-end completion and turn flows.

5. Full package tests:
- `cargo test -p loongclaw-app`
- `cargo test -p loongclaw-kernel`
- `cargo test -p loongclaw-daemon`
