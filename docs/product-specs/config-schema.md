# Configuration Schema

This document describes the complete configuration schema for LoongClaw. Configuration is stored in TOML format.

`loongclaw validate-config` performs structural and range diagnostics for provider/channels/tools/memory/feishu_integration fields. Some fields (notably parts of `conversation` and most `acp` runtime knobs) are accepted and then normalized at runtime rather than rejected by validate diagnostics.

## Configuration File Location

The default configuration file is located at:

- **Linux/macOS**: `~/.loongclaw/config.toml`
- **Windows**: `%USERPROFILE%\.loongclaw\config.toml`

You can specify a custom path with the `--config` flag:

```bash
loongclaw validate-config --config /path/to/custom-config.toml
```

## Configuration Sections

- [top-level](#top-level) - Global selector and runtime state fields
- [provider](#provider) - LLM provider settings
- [providers](#providers) - Multiple provider profiles
- [tools](#tools) - Tool execution settings
- [memory](#memory) - Memory and conversation history
- [conversation](#conversation) - Conversation runtime behavior
- [acp](#acp) - Agent Control Plane settings
- [external_skills](#external_skills) - External skill management
- [audit](#audit) - Audit logging
- [channels](#channels) - Channel-specific settings

## Validation Semantics

- **Validate-config diagnostics**: Fields that produce explicit validation diagnostics (error/warn) during `loongclaw validate-config`.
- **Runtime-normalized**: Fields that are accepted at parse/validate time and clamped/defaulted by runtime resolution logic.
- **Provider/runtime-dependent**: Fields whose effective behavior depends on provider capability or runtime routing.

Unless explicitly noted otherwise, `Valid Values / Range` means the expected operational domain for that field.

---

## top-level

Top-level fields that coordinate provider profile selection and runtime state.

| Field | Type | Default | Valid Values / Typical Range | Description |
|-------|------|---------|---------------------------|-------------|
| `active_provider` | string? | `null` | Provider profile ID (`[a-zA-Z0-9_-]+`) | Explicit active provider profile selector (preferred in multi-provider setups) |
| `last_provider` | string? | `null` | Provider profile ID (`[a-zA-Z0-9_-]+`) | Last selected provider profile ID (runtime state) |

Notes:

- In single-provider setups, the legacy `[provider]` table can still be used.
- In multi-provider setups, `[providers.<profile_id>]` plus `active_provider` is the canonical model.

---

## provider

Configuration for the LLM provider.

| Field | Type | Default | Valid Values / Typical Range | Description |
|-------|------|---------|---------------------------|-------------|
| `kind` | string | `"openai"` | See [Provider Kinds](#provider-kinds) | Provider type |
| `model` | string | `"auto"` | Model name / `"auto"` | Model name or "auto" for discovery |
| `base_url` | string | Provider-specific | URL | API base URL |
| `wire_api` | string | `"chat_completions"` | `"chat_completions"`, `"responses"` | Wire API format |
| `chat_completions_path` | string | `"/v1/chat/completions"` | Path | Chat completions endpoint path |
| `endpoint` | string? | `null` | URL | Override complete endpoint URL |
| `models_endpoint` | string? | `null` | URL | Override models list endpoint |
| `api_key` | string? | `null` | API key string | API key (use env reference preferred) |
| `api_key_env` | string? | `null` | Env var name | Environment variable containing API key |
| `oauth_access_token` | string? | `null` | Token string | OAuth access token |
| `oauth_access_token_env` | string? | `null` | Env var name | Environment variable containing OAuth token |
| `preferred_models` | string[] | `[]` | Model names | Fallback models when discovery fails |
| `reasoning_effort` | string? | `null` | `"none"`, `"minimal"`, `"low"`, `"medium"`, `"high"`, `"xhigh"` | Reasoning effort level |
| `headers` | map | `{}` | HTTP headers | Custom headers to add to requests |
| `temperature` | float | `0.2` | `0.0` - `2.0` | Sampling temperature |
| `max_tokens` | integer? | `null` | `1`+ | Maximum tokens per response |
| `request_timeout_ms` | integer | `30000` | `1`+ | Request timeout in milliseconds |
| `retry_max_attempts` | integer | `3` | `0`+ | Maximum retry attempts |
| `retry_initial_backoff_ms` | integer | `300` | `0`+ | Initial retry backoff |
| `retry_max_backoff_ms` | integer | `3000` | `1`+ | Maximum retry backoff |
| `model_catalog_cache_ttl_ms` | integer | `30000` | `0`+ | Model catalog cache TTL |
| `model_catalog_stale_if_error_ms` | integer | `120000` | `0`+ | Stale cache fallback TTL |
| `model_catalog_cache_max_entries` | integer | `32` | `1`+ | Max catalog cache entries |
| `model_candidate_cooldown_ms` | integer | `300000` | `0`+ | Model failure cooldown |
| `model_candidate_cooldown_max_ms` | integer | `3600000` | `0`+ | Max model cooldown |
| `model_candidate_cooldown_max_entries` | integer | `64` | `1`+ | Max cooldown tracking entries |
| `profile_cooldown_ms` | integer | `60000` | `0`+ | Profile failure cooldown |
| `profile_cooldown_max_ms` | integer | `3600000` | `0`+ | Max profile cooldown |
| `profile_auth_reject_disable_ms` | integer | `21600000` | `1`+ | Auth rejection disable duration |
| `profile_state_max_entries` | integer | `256` | `1`+ | Max profile state entries |
| `profile_state_backend` | string | `"file"` | `"file"`, `"sqlite"` | Profile state storage backend |
| `profile_state_sqlite_path` | string? | `null` | Path | SQLite database path |
| `profile_health_mode` | string | `"provider_default"` | `"provider_default"`, `"enforce"`, `"observe_only"` | Health check mode |
| `tool_schema_mode` | string | `"provider_default"` | `"provider_default"`, `"disabled"`, `"enabled_strict"`, `"enabled_with_downgrade"` | Tool schema handling |
| `reasoning_extra_body_mode` | string | `"provider_default"` | `"provider_default"`, `"omit"`, `"kimi_thinking"` | Reasoning extra body handling |
| `tool_schema_disabled_model_hints` | string[] | `[]` | Model names | Models that don't support tool schema |
| `tool_schema_strict_model_hints` | string[] | `[]` | Model names | Models requiring strict tool schema |
| `reasoning_extra_body_kimi_model_hints` | string[] | `[]` | Model names | Models supporting Kimi thinking format |
| `reasoning_extra_body_omit_model_hints` | string[] | `[]` | Model names | Models that should omit reasoning extra body |

### Provider Kinds

| Kind | Aliases | Default Base URL |
|------|---------|------------------|
| `openai` | `openai_compatible` | `https://api.openai.com` |
| `anthropic` | `anthropic_compatible` | `https://api.anthropic.com` |
| `kimi` | `moonshot`, `moonshot_compatible`, `kimi_compatible` | `https://api.moonshot.cn` |
| `kimi_coding` | `kimi_coding_compatible` | `https://api.kimi.com` |
| `deepseek` | `deepseek_compatible` | `https://api.deepseek.com` |
| `volcengine` | `volcengine_compatible`, `volcengine_custom`, `doubao`, `ark` | `https://ark.cn-beijing.volces.com` |
| `volcengine_coding` | `volcengine_coding_compatible` | `https://ark.cn-beijing.volces.com/api/coding/v3` |
| `bailian_coding` | `bailian_coding_compatible` | `https://coding.dashscope.aliyuncs.com/v1` |
| `byteplus` | `byteplus_compatible` | `https://ark.ap-southeast.bytepluses.com/api/v3` |
| `byteplus_coding` | `byteplus_coding_compatible` | `https://ark.ap-southeast.bytepluses.com/api/coding/v3` |
| `ollama` | `ollama_compatible` | `http://127.0.0.1:11434` |
| `openrouter` | `openrouter_compatible` | `https://openrouter.ai` |
| `groq` | `groq_compatible` | `https://api.groq.com` |
| `fireworks` | `fireworks_compatible` | `https://api.fireworks.ai` |
| `mistral` | `mistral_compatible` | `https://api.mistral.ai` |
| `minimax` | `minimax_compatible` | `https://api.minimaxi.com` |
| `gemini` | `google`, `google_gemini`, `gemini_compatible` | `https://generativelanguage.googleapis.com/v1beta/openai` |
| `bedrock` | `aws_bedrock`, `aws-bedrock` | `https://bedrock-runtime.<region>.amazonaws.com` |
| `cohere` | `cohere_compatible` | `https://api.cohere.ai/compatibility` |
| `cerebras` | `cerebras_compatible` | `https://api.cerebras.ai` |
| `cloudflare_ai_gateway` | `cloudflare_ai`, `cloudflare-ai`, `cloudflare_ai_gateway`, `cloudflare-ai-gateway` | `https://gateway.ai.cloudflare.com/v1/<account_id>/<gateway_name>/openai/compat` |
| `novita` | `novita_compatible` | `https://api.novita.ai` |
| `nvidia` | `nvidia_compatible`, `nvidia_nim` | `https://integrate.api.nvidia.com` |
| `llamacpp` | `llama.cpp`, `llama_cpp` | `http://127.0.0.1:8080` |
| `lm_studio` | `lmstudio`, `lm-studio` | `http://127.0.0.1:1234` |
| `perplexity` | `perplexity_compatible` | `https://api.perplexity.ai` |
| `qianfan` | `qianfan_compatible`, `baidu` | `https://qianfan.baidubce.com` |
| `qwen` | `qwen_compatible`, `dashscope` | `https://dashscope.aliyuncs.com` |
| `sambanova` | `sambanova_compatible`, `samba_nova` | `https://api.sambanova.ai` |
| `sglang` | `sglang_compatible` | `http://127.0.0.1:30000` |
| `siliconflow` | `siliconflow_compatible` | `https://api.siliconflow.com` |
| `stepfun` | `stepfun_compatible` | `https://api.stepfun.com` |
| `together` | `together_compatible`, `together_ai` | `https://api.together.xyz` |
| `venice` | `venice_compatible` | `https://api.venice.ai` |
| `vercel_ai_gateway` | `vercel_ai`, `vercel-ai`, `vercel_ai_gateway`, `vercel-ai-gateway` | `https://ai-gateway.vercel.sh/v1` |
| `xai` | `xai_compatible` | `https://api.x.ai` |
| `zai` | `zai_compatible` | `https://api.z.ai` |
| `zhipu` | `zhipu_compatible` | `https://open.bigmodel.cn` |
| `vllm` | `vllm_compatible` | `http://127.0.0.1:8000` |
| `custom` | `openai_custom`, `custom_openai` | `https://<openai-compatible-host>/v1` |

### Example: OpenAI

```toml
[provider]
kind = "openai"
model = "gpt-4o"
api_key = "${OPENAI_API_KEY}"
temperature = 0.7
max_tokens = 4096
```

### Example: Kimi

```toml
[provider]
kind = "kimi"
model = "kimi-k2-0725-preview"
api_key = "${MOONSHOT_API_KEY}"
```

### Example: Local Ollama

```toml
[provider]
kind = "ollama"
model = "llama3.2"
base_url = "http://localhost:11434"
```

---

## providers

Map of named provider profiles for multi-provider configurations.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `[providers.<profile_id>]` | table | - | - | Provider profile configuration |
| `default_for_kind` | boolean | `false` | `true`, `false` | Whether this is the default for its provider kind |

All fields from [provider](#provider) are available under each profile.

### Example

```toml
[providers.openai]
default_for_kind = true
kind = "openai"
model = "gpt-4o"
api_key = "${OPENAI_API_KEY}"

[providers.kimi_pro]
kind = "kimi"
model = "kimi-k2-0725-preview"
api_key = "${MOONSHOT_API_KEY}"
```

---

## tools

Configuration for tool execution and approval policies.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `file_root` | string? | Current directory | Path | Root directory for file operations |
| `shell_allow` | string[] | `[]` | Command names | Allowed shell commands |
| `shell_deny` | string[] | `[]` | Command names | Denied shell commands |
| `shell_default_mode` | string | `"deny"` | `"deny"`, `"allow"` | Default for unknown commands |

Notes:

- `tools` includes strict numeric/domain diagnostics for browser/web/web_search/delegate child runtime limits.
- `shell_default_mode` is documented with supported operational values (`"deny"`, `"allow"`).

### tools.approval

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `mode` | string | `"disabled"` | `"disabled"`, `"medium_balanced"`, `"strict"` | Tool approval mode |
| `approved_calls` | string[] | `[]` | Tool call patterns | Pre-approved tool calls |
| `denied_calls` | string[] | `[]` | Tool call patterns | Denied tool calls |

### tools.sessions

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable session management tools |
| `visibility` | string | `"children"` | `"self"`, `"children"` | Session visibility |
| `list_limit` | integer | `100` | `1`+ | Maximum sessions in list |
| `history_limit` | integer | `200` | `1`+ | Maximum messages in history |

### tools.messages

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable message management tools |

### tools.delegate

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable delegate subagent tool |
| `max_depth` | integer | `1` | `1`+ | Maximum delegation depth |
| `max_active_children` | integer | `5` | `1`+ | Maximum concurrent child delegates |
| `timeout_seconds` | integer | `60` | `1`+ | Delegate timeout |
| `child_tool_allowlist` | string[] | `["file.read", "file.write"]` | Tool names | Tools allowed in children |
| `allow_shell_in_child` | boolean | `false` | `true`, `false` | Allow shell commands in children |

#### tools.delegate.child_runtime.web

| Field | Type | Default | Valid Range | Description |
|-------|------|---------|-------------|-------------|
| `allow_private_hosts` | boolean? | `null` | `true`, `false` | Allow private network hosts |
| `allowed_domains` | string[] | `[]` | Domain names | Allowed domain list |
| `blocked_domains` | string[] | `[]` | Domain names | Blocked domain list |
| `timeout_seconds` | integer? | `null` | `1` - `120` | Web fetch timeout override |
| `max_bytes` | integer? | `null` | `1024` - `5242880` | Max response bytes override |
| `max_redirects` | integer? | `null` | `0` - `10` | Max redirects override |

#### tools.delegate.child_runtime.browser

| Field | Type | Default | Valid Range | Description |
|-------|------|---------|-------------|-------------|
| `max_sessions` | integer? | `null` | `1` - `32` | Max browser sessions override |
| `max_links` | integer? | `null` | `1` - `200` | Max links per page override |
| `max_text_chars` | integer? | `null` | `256` - `20000` | Max text extraction chars override |

### tools.browser

| Field | Type | Default | Valid Range/Values | Description |
|-------|------|---------|-------------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable browser automation |
| `max_sessions` | integer | `8` | `1` - `32` | Maximum concurrent browser sessions |
| `max_links` | integer | `40` | `1` - `200` | Maximum links per page |
| `max_text_chars` | integer | `6000` | `256` - `20000` | Maximum text extraction characters |

### tools.browser_companion

| Field | Type | Default | Valid Range/Values | Description |
|-------|------|---------|-------------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable browser companion |
| `command` | string? | `null` | Path | Browser companion command path |
| `expected_version` | string? | `null` | Version string | Expected companion version |
| `timeout_seconds` | integer | `30` | `1`+ | Companion timeout (seconds) |

### tools.web

| Field | Type | Default | Valid Range/Values | Description |
|-------|------|---------|-------------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable web fetch |
| `allow_private_hosts` | boolean | `false` | `true`, `false` | Allow private network access |
| `allowed_domains` | string[] | `[]` | Domain names | Allowed domain whitelist |
| `blocked_domains` | string[] | `[]` | Domain names | Blocked domain blacklist |
| `max_bytes` | integer | `1048576` | `1024` - `5242880` | Maximum response size (bytes) |
| `timeout_seconds` | integer | `15` | `1` - `120` | Request timeout |
| `max_redirects` | integer | `3` | `0` - `10` | Maximum redirects to follow |

### tools.web_search

| Field | Type | Default | Valid Range/Values | Description |
|-------|------|---------|-------------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable web search |
| `default_provider` | string | `"duckduckgo"` | `"duckduckgo"`, `"ddg"`, `"brave"`, `"tavily"` | Search provider |
| `timeout_seconds` | integer | `30` | `1` - `60` | Search timeout |
| `max_results` | integer | `5` | `1` - `10` | Maximum results |
| `brave_api_key` | string? | `null` | API key string | Brave Search API key |
| `tavily_api_key` | string? | `null` | API key string | Tavily API key |

### Example

```toml
[tools]
shell_allow = ["git", "cargo", "npm"]
shell_default_mode = "deny"

[tools.approval]
mode = "medium_balanced"
approved_calls = ["tool:file_read"]

[tools.browser]
max_sessions = 8
max_links = 40

[tools.web_search]
default_provider = "brave"
brave_api_key = "${BRAVE_API_KEY}"
```

---

## memory

Configuration for conversation memory and history management.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `backend` | string | `"sqlite"` | `"sqlite"` | Storage backend |
| `profile` | string | `"window_only"` | `"window_only"`, `"window_plus_summary"`, `"profile_plus_window"` | Memory mode profile |
| `system` | string | `"builtin"` | `"builtin"` | Memory system implementation |
| `fail_open` | boolean | `true` | `true`, `false` | Allow fallback on memory errors |
| `ingest_mode` | string | `"sync_minimal"` | `"sync_minimal"`, `"async_background"` | Message ingestion mode |
| `sqlite_path` | string | `~/.loongclaw/memory.sqlite3` | Path | SQLite database location |
| `sliding_window` | integer | `12` | `1` - `128` | Turns to retain in window |
| `summary_max_chars` | integer | `1200` | `256`+ (effective floor) | Summary character budget |
| `profile_note` | string? | `null` | Text | Optional profile description |

Note: `memory.summary_max_chars` values below `256` are accepted but normalized to an effective minimum of `256` at runtime.

### Memory Profiles

| Profile | Description |
|---------|-------------|
| `window_only` | Retain only recent messages in sliding window |
| `window_plus_summary` | Window + compressed summary of older messages |
| `profile_plus_window` | User profile + recent window |

### Example

```toml
[memory]
profile = "window_plus_summary"
sliding_window = 24
summary_max_chars = 2000
sqlite_path = "~/.loongclaw/memory.sqlite3"
```

---

## conversation

Configuration for conversation runtime behavior.

Notes:

- Several `conversation` fields are runtime-normalized (for example clamped thresholds and minimum floors) instead of emitting validate-config errors.

| Field | Type | Default | Valid Range/Values | Description |
|-------|------|---------|-------------------|-------------|
| `context_engine` | string? | `null` | Engine ID | Context engine ID |
| `turn_middlewares` | string[] | `[]` | Middleware IDs | Active middleware chain |
| `compact_enabled` | boolean | `true` | `true`, `false` | Enable conversation compaction |
| `compact_min_messages` | integer? | `null` | `1`+ | Messages threshold for compaction |
| `compact_trigger_estimated_tokens` | integer? | `null` | `1`+ | Token threshold for compaction |
| `compact_fail_open` | boolean | `true` | `true`, `false` | Allow fallback on compaction errors |
| `hybrid_lane_enabled` | boolean | `true` | `true`, `false` | Enable fast/safe lane routing |
| `safe_lane_plan_execution_enabled` | boolean | `false` | `true`, `false` | Enable plan-based execution |
| `fast_lane_max_tool_steps_per_turn` | integer | `1` | `1`+ | Max tool calls per fast lane turn |
| `fast_lane_parallel_tool_execution_enabled` | boolean | `false` | `true`, `false` | Enable parallel tool execution |
| `fast_lane_parallel_tool_execution_max_in_flight` | integer | `4` | `1`+ | Max parallel tool calls |
| `safe_lane_max_tool_steps_per_turn` | integer | `1` | `1`+ | Max tool calls per safe lane turn |
| `safe_lane_node_max_attempts` | integer | `2` | `1`+ | Max retries per plan node |
| `safe_lane_plan_max_wall_time_ms` | integer | `30000` | `1`+ | Max plan execution time (ms) |
| `safe_lane_verify_output_non_empty` | boolean | `true` | `true`, `false` | Require non-empty output |
| `safe_lane_verify_min_output_chars` | integer | `8` | `1`+ | Minimum output length |
| `safe_lane_verify_require_status_prefix` | boolean | `true` | `true`, `false` | Require status prefix |
| `safe_lane_verify_adaptive_anchor_escalation` | boolean | `true` | `true`, `false` | Enable adaptive escalation |
| `safe_lane_verify_anchor_escalation_after_failures` | integer | `2` | `1`+ | Escalation threshold |
| `safe_lane_verify_anchor_escalation_min_matches` | integer | `1` | `1`+ | Min anchor matches |
| `safe_lane_emit_runtime_events` | boolean | `true` | `true`, `false` | Emit runtime events |
| `safe_lane_event_sample_every` | integer | `1` | `1`+ | Event sampling rate |
| `safe_lane_event_adaptive_sampling` | boolean | `true` | `true`, `false` | Adaptive event sampling |
| `safe_lane_event_adaptive_failure_threshold` | integer | `1` | `1`+ | Failure threshold for sampling |
| `safe_lane_verify_deny_markers` | string[] | See below | Marker strings | Deny markers for verification |
| `safe_lane_replan_max_rounds` | integer | `1` | `1`+ | Max replan iterations |
| `safe_lane_replan_max_node_attempts` | integer | `4` | `1`+ | Max node attempts per replan |
| `safe_lane_session_governor_enabled` | boolean | `true` | `true`, `false` | Enable session governor |
| `safe_lane_session_governor_window_turns` | integer | `96` | `1`+ | Governor window size |
| `safe_lane_session_governor_failed_final_status_threshold` | integer | `3` | `1`+ | Failure threshold |
| `safe_lane_session_governor_backpressure_failure_threshold` | integer | `1` | `1`+ | Backpressure threshold |
| `safe_lane_session_governor_trend_enabled` | boolean | `true` | `true`, `false` | Enable trend analysis |
| `safe_lane_session_governor_trend_min_samples` | integer | `4` | `1`+ | Min samples for trend |
| `safe_lane_session_governor_trend_ewma_alpha` | float | `0.35` | `0.01` - `1.0` | EWMA smoothing factor |
| `safe_lane_session_governor_trend_failure_ewma_threshold` | float | `0.60` | `0.0` - `1.0` | Failure EWMA threshold |
| `safe_lane_session_governor_trend_backpressure_ewma_threshold` | float | `0.20` | `0.0` - `1.0` | Backpressure EWMA threshold |
| `safe_lane_session_governor_recovery_success_streak` | integer | `3` | `1`+ | Success streak for recovery |
| `safe_lane_session_governor_recovery_max_failure_ewma` | float | `0.25` | `0.0` - `1.0` | Max failure EWMA for recovery |
| `safe_lane_session_governor_recovery_max_backpressure_ewma` | float | `0.10` | `0.0` - `1.0` | Max backpressure for recovery |
| `safe_lane_session_governor_force_no_replan` | boolean | `true` | `true`, `false` | Force no replan in recovery |
| `safe_lane_session_governor_force_node_max_attempts` | integer | `1` | `1`+ | Force node attempts in recovery |
| `safe_lane_backpressure_guard_enabled` | boolean | `true` | `true`, `false` | Enable backpressure guard |
| `safe_lane_backpressure_max_total_attempts` | integer | `32` | `1`+ | Max total attempts |
| `safe_lane_backpressure_max_replans` | integer | `8` | `1`+ | Max replans |
| `safe_lane_risk_threshold` | integer | `4` | `1`+ | Risk assessment threshold |
| `safe_lane_complexity_threshold` | integer | `6` | `1`+ | Complexity threshold |
| `fast_lane_max_input_chars` | integer | `400` | `1`+ | Max input characters |
| `tool_result_payload_summary_limit_chars` | integer | `2048` | `256` - `64000` | Tool result summary limit |
| `safe_lane_health_truncation_warn_threshold` | float | `0.30` | `0.0` - `1.0` | Truncation warning threshold |
| `safe_lane_health_truncation_critical_threshold` | float | `0.60` | `0.0` - `1.0` | Truncation critical threshold |
| `safe_lane_health_verify_failure_warn_threshold` | float | `0.40` | `0.0` - `1.0` | Verify failure warning threshold |
| `safe_lane_health_replan_warn_threshold` | float | `0.50` | `0.0` - `1.0` | Replan warning threshold |
| `high_risk_keywords` | string[] | See below | Keywords | High-risk keyword list |

Note: `conversation.tool_result_payload_summary_limit_chars` is normalized to an effective runtime range of `256` to `64000`.

### conversation.turn_loop

| Field | Type | Default | Valid Range | Description |
|-------|------|---------|-------------|-------------|
| `max_rounds` | integer | `4` | `1`+ | Maximum conversation rounds |
| `max_tool_steps_per_round` | integer | `1` | `1`+ | Max tool calls per round |
| `max_repeated_tool_call_rounds` | integer | `2` | `1`+ | Max repeated tool calls |
| `max_ping_pong_cycles` | integer | `2` | `1`+ | Max ping-pong cycles |
| `max_same_tool_failure_rounds` | integer | `3` | `1`+ | Max failures for same tool |
| `max_followup_tool_payload_chars` | integer | `8000` | `1`+ | Max followup payload |
| `max_followup_tool_payload_chars_total` | integer | `20000` | `1`+ | Max total followup payload |
| `max_discovery_followup_rounds` | integer | `2` | `1`+ | Max discovery rounds |

### Default Deny Markers

```toml
safe_lane_verify_deny_markers = [
  "tool_failure",
  "provider_error",
  "no_kernel_context",
  "tool_not_found"
]
```

### Default High-Risk Keywords

```toml
high_risk_keywords = [
  "rm -rf",
  "drop table",
  "delete",
  "credential",
  "token",
  "secret",
  "prod",
  "production",
  "deploy",
  "payment",
  "wallet"
]
```

### Example

```toml
[conversation]
compact_enabled = true
fast_lane_max_tool_steps_per_turn = 2
safe_lane_max_tool_steps_per_turn = 1

[conversation.turn_loop]
max_rounds = 6
max_tool_steps_per_round = 2
```

---

## acp

Configuration for Agent Control Plane (ACP).

Notes:

- ACP identifiers are normalized and checked when dispatch/session logic resolves them.
- Most ACP numeric controls are runtime-resolved (for example fallback to defaults when unset or non-positive) rather than validated as numeric-range diagnostics by `validate-config`.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable ACP |
| `backend` | string? | `null` | Backend ID | Backend implementation ID |
| `default_agent` | string? | `null` | Agent ID | Default agent ID |
| `allowed_agents` | string[] | `[]` | Agent IDs | Allowed agent IDs |
| `max_concurrent_sessions` | integer? | `8` | `1`+ | Max concurrent sessions |
| `session_idle_ttl_ms` | integer? | `900000` | `1`+ | Session idle TTL (ms) |
| `startup_timeout_ms` | integer? | `15000` | `1`+ | Startup timeout (ms) |
| `turn_timeout_ms` | integer? | `120000` | `1`+ | Turn timeout (ms) |
| `queue_owner_ttl_ms` | integer? | `30000` | `1`+ | Queue owner TTL (ms) |
| `bindings_enabled` | boolean | `false` | `true`, `false` | Enable bindings |
| `emit_runtime_events` | boolean | `false` | `true`, `false` | Emit runtime events |
| `allow_mcp_server_injection` | boolean | `false` | `true`, `false` | Allow MCP server injection |

### acp.dispatch

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable dispatch |
| `conversation_routing` | string | `"agent_prefixed_only"` | `"agent_prefixed_only"`, `"all"` | Routing mode |
| `allowed_channels` | string[] | `[]` | Channel IDs | Allowed channel IDs |
| `allowed_account_ids` | string[] | `[]` | Account IDs | Allowed account IDs |
| `bootstrap_mcp_servers` | string[] | `[]` | Server names | Auto-start MCP servers |
| `working_directory` | string? | `null` | Path | Default working directory |
| `thread_routing` | string | `"all"` | `"all"`, `"thread_only"`, `"root_only"` | Thread routing mode |

### acp.backends.acpx

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `command` | string? | `null` | Command path | Backend command |
| `expected_version` | string? | `null` | Version string | Expected version |
| `cwd` | string? | `null` | Path | Working directory |
| `permission_mode` | string? | `null` | Permission mode string | Permission mode |
| `non_interactive_permissions` | string? | `null` | Permissions string | Non-interactive permissions |
| `strict_windows_cmd_wrapper` | boolean? | `null` | `true`, `false` | Strict Windows wrapper |
| `timeout_seconds` | float? | `null` | `> 0` | Timeout (seconds) |
| `queue_owner_ttl_seconds` | float? | `null` | `>= 0` | Queue TTL (seconds) |

#### acp.backends.acpx.mcp_servers

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `[acp.backends.acpx.mcp_servers.<name>]` | table | - | - | MCP server configuration |
| `command` | string | Required | Command path | Server command |
| `args` | string[] | `[]` | Arguments | Command arguments |
| `env` | map | `{}` | Key-value pairs | Environment variables |

### Example

```toml
[acp]
enabled = true
backend = "acpx"
default_agent = "codex"
allowed_agents = ["codex", "custom"]

[acp.dispatch]
conversation_routing = "agent_prefixed_only"
allowed_channels = ["cli", "telegram"]

[acp.backends.acpx]
command = "acpx-server"

[acp.backends.acpx.mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/home/user"]
```

---

## external_skills

Configuration for external skill management.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable external skills |
| `require_download_approval` | boolean | `true` | `true`, `false` | Require approval for downloads |
| `allowed_domains` | string[] | `[]` | Domain names | Allowed download domains |
| `blocked_domains` | string[] | `[]` | Domain names | Blocked download domains |
| `install_root` | string? | `null` | Path | Custom install directory |
| `auto_expose_installed` | boolean | `false` | `true`, `false` | Auto-expose installed skills |

### Example

```toml
[external_skills]
enabled = true
require_download_approval = true
allowed_domains = ["github.com", "gist.github.com"]
```

---

## audit

Configuration for audit logging.

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `mode` | string | `"fanout"` | `"in_memory"`, `"jsonl"`, `"fanout"` | Audit mode |
| `path` | string | `~/.loongclaw/audit/events.jsonl` | Path | Log file path |
| `retain_in_memory` | boolean | `true` | `true`, `false` | Keep events in memory |

### Audit Modes

| Mode | Description |
|------|-------------|
| `in_memory` | Events stored only in memory |
| `jsonl` | Events written to JSONL file |
| `fanout` | Events sent to multiple sinks |

### Example

```toml
[audit]
mode = "jsonl"
path = "~/.loongclaw/audit/events.jsonl"
```

---

## channels

Configuration for input/output channels.

### cli

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `true` | `true`, `false` | Enable CLI channel |
| `system_prompt` | string | Default prompt | Prompt text | System prompt template |
| `prompt_pack_id` | string? | `"loongclaw-core-v1"` | Pack ID | Prompt pack ID |
| `personality` | string? | `"calm_engineering"` | `"calm_engineering"`, `"friendly_collab"`, `"autonomous_executor"` | Personality preset |
| `system_prompt_addendum` | string? | `null` | Prompt text | Additional prompt text |
| `exit_commands` | string[] | `["/exit", "/quit"]` | Commands | Commands to exit chat |

### telegram

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable Telegram bot |
| `account_id` | string? | `null` | Account ID | Bot account identifier |
| `default_account` | string? | `null` | Account ID | Default account |
| `bot_token` | string? | `null` | Token string | Bot token (use env preferred) |
| `bot_token_env` | string? | `"TELEGRAM_BOT_TOKEN"` | Env var name | Env var for bot token |
| `base_url` | string | `"https://api.telegram.org"` | URL | API base URL |
| `polling_timeout_s` | integer | `15` | `1`+ | Polling timeout (seconds) |
| `allowed_chat_ids` | integer[] | `[]` | Chat IDs | Allowed chat IDs |
| `acp` | table | `{}` | ACP config table | ACP overrides for this channel |
| `accounts` | map | `{}` | Account ID -> account config | Per-account overrides |

#### telegram.accounts

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean? | `null` | `true`, `false` | Whether account is enabled |
| `account_id` | string? | `null` | Account ID | Account identifier |
| `bot_token` | string? | `null` | Token string | Account bot token |
| `bot_token_env` | string? | `null` | Env var name | Env var for token |
| `base_url` | string? | `null` | URL | API base URL override |
| `polling_timeout_s` | integer? | `null` | `1`+ | Polling timeout override |
| `allowed_chat_ids` | integer[]? | `null` | Chat IDs | Allowed chat IDs |
| `acp` | table? | `null` | - | ACP configuration |

### feishu

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable Feishu bot |
| `account_id` | string? | `null` | Account ID | App account identifier |
| `default_account` | string? | `null` | Account ID | Default account |
| `app_id` | string? | `null` | App ID | App ID |
| `app_id_env` | string? | `"FEISHU_APP_ID"` | Env var name | Env var for app ID |
| `app_secret` | string? | `null` | Secret string | App secret |
| `app_secret_env` | string? | `"FEISHU_APP_SECRET"` | Env var name | Env var for app secret |
| `verification_token` | string? | `null` | Token string | Verification token |
| `verification_token_env` | string? | `"FEISHU_VERIFICATION_TOKEN"` | Env var name | Env var for verification token |
| `encrypt_key` | string? | `null` | Key string | Encryption key |
| `encrypt_key_env` | string? | `"FEISHU_ENCRYPT_KEY"` | Env var name | Env var for encrypt key |
| `domain` | string | `"feishu"` | `"feishu"`, `"lark"` | Domain |
| `base_url` | string? | `null` | URL | API base URL override |
| `mode` | string? | `null` | `"webhook"`, `"websocket"` | Serve mode |
| `receive_id_type` | string | `"chat_id"` | ID type string | Receive ID type |
| `webhook_bind` | string | `"127.0.0.1:8080"` | Address | Webhook bind address |
| `webhook_path` | string | `"/feishu/events"` | Path | Webhook path |
| `allowed_chat_ids` | string[] | `[]` | Chat IDs | Allowed chat IDs |
| `ignore_bot_messages` | boolean | `true` | `true`, `false` | Ignore bot messages |
| `acp` | table | `{}` | ACP config table | ACP overrides for this channel |
| `accounts` | map | `{}` | Account ID -> account config | Per-account overrides |

#### feishu.accounts

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean? | `null` | `true`, `false` | Whether account is enabled |
| `account_id` | string? | `null` | Account ID | Account identifier |
| `app_id` | string? | `null` | App ID | App ID |
| `app_id_env` | string? | `null` | Env var name | Env var for app ID |
| `app_secret` | string? | `null` | Secret string | App secret |
| `app_secret_env` | string? | `null` | Env var name | Env var for app secret |
| `domain` | string? | `null` | `"feishu"`, `"lark"` | Domain override |
| `base_url` | string? | `null` | URL | Base URL override |
| `mode` | string? | `null` | `"webhook"`, `"websocket"` | Serve mode |
| `receive_id_type` | string? | `null` | ID type | Receive ID type |
| `webhook_bind` | string? | `null` | Address | Webhook bind override |
| `webhook_path` | string? | `null` | Path | Webhook path override |
| `verification_token` | string? | `null` | Token string | Verification token |
| `verification_token_env` | string? | `null` | Env var name | Env var for token |
| `encrypt_key` | string? | `null` | Key string | Encryption key |
| `encrypt_key_env` | string? | `null` | Env var name | Env var for encrypt key |
| `allowed_chat_ids` | string[]? | `null` | Chat IDs | Allowed chat IDs |
| `ignore_bot_messages` | boolean? | `null` | `true`, `false` | Ignore bot messages override |
| `acp` | table? | `null` | - | ACP configuration |

### matrix

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean | `false` | `true`, `false` | Enable Matrix bot |
| `account_id` | string? | `null` | Account ID | Account identifier |
| `default_account` | string? | `null` | Account ID | Default account |
| `user_id` | string? | `null` | User ID | Matrix user ID |
| `access_token` | string? | `null` | Token string | Access token |
| `access_token_env` | string? | `"MATRIX_ACCESS_TOKEN"` | Env var name | Env var for token |
| `base_url` | string? | `null` | URL | Homeserver URL |
| `sync_timeout_s` | integer | `30` | `1`+ | Sync timeout (seconds) |
| `allowed_room_ids` | string[] | `[]` | Room IDs | Allowed room IDs |
| `ignore_self_messages` | boolean | `true` | `true`, `false` | Ignore own messages |
| `acp` | table | `{}` | - | ACP configuration |
| `accounts` | map | `{}` | - | Account configurations |

#### matrix.accounts

| Field | Type | Default | Valid Values | Description |
|-------|------|---------|--------------|-------------|
| `enabled` | boolean? | `null` | `true`, `false` | Whether account is enabled |
| `account_id` | string? | `null` | Account ID | Account identifier |
| `user_id` | string? | `null` | User ID | Matrix user ID |
| `access_token` | string? | `null` | Token string | Access token |
| `access_token_env` | string? | `null` | Env var name | Env var for token |
| `base_url` | string? | `null` | URL | Homeserver URL |
| `sync_timeout_s` | integer? | `null` | `1`+ | Sync timeout override |
| `allowed_room_ids` | string[]? | `null` | Room IDs | Allowed room IDs |
| `ignore_self_messages` | boolean? | `null` | `true`, `false` | Ignore own messages override |
| `acp` | table? | `null` | - | ACP configuration |

### Example

```toml
[cli]
system_prompt = "You are a helpful assistant."
exit_commands = ["exit", "quit", "bye"]

[telegram]
enabled = true
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_chat_ids = [123456789]

[feishu]
enabled = true
app_id_env = "FEISHU_APP_ID"
app_secret_env = "FEISHU_APP_SECRET"
```

---

## feishu_integration

Configuration for Feishu OAuth integration.

| Field | Type | Default | Valid Range | Description |
|-------|------|---------|-------------|-------------|
| `sqlite_path` | string | `~/.loongclaw/feishu.sqlite3` | Path | SQLite database path |
| `oauth_state_ttl_s` | integer | `600` | `60` - `86400` | OAuth state TTL |
| `request_timeout_s` | integer | `20` | `3` - `120` | Request timeout |
| `retry_max_attempts` | integer | `4` | `1` - `8` | Max retry attempts |
| `retry_initial_backoff_ms` | integer | `200` | `0` - `30000` | Initial backoff |
| `retry_max_backoff_ms` | integer | `2000` | `retry_initial_backoff_ms` - `60000` | Max backoff |
| `default_scopes` | string[] | See below | OAuth scopes | Default OAuth scopes |

### Default Scopes

```toml
default_scopes = [
  "offline_access",
  "docx:document:readonly",
  "im:message:readonly",
  "im:message.group_msg:readonly",
  "search:message",
  "calendar:calendar:readonly"
]
```

---

## Complete Example Configurations

### Minimal OpenAI Setup

```toml
[provider]
kind = "openai"
model = "gpt-4o"
api_key = "${OPENAI_API_KEY}"
```

### Development Environment with Ollama

```toml
[provider]
kind = "ollama"
model = "llama3.2"
base_url = "http://localhost:11434"

[tools]
shell_allow = ["git", "cargo", "make"]
shell_default_mode = "deny"

[memory]
profile = "window_only"
sliding_window = 24
```

### Multi-Provider Production Setup

```toml
active_provider = "openai_prod"

[providers.openai_prod]
default_for_kind = true
kind = "openai"
model = "gpt-4o"
api_key = "${OPENAI_API_KEY}"
request_timeout_ms = 90000

[providers.kimi_backup]
kind = "kimi"
model = "kimi-k2-0725-preview"
api_key = "${MOONSHOT_API_KEY}"

[tools]
shell_allow = ["git", "docker"]

[tools.approval]
mode = "strict"

[memory]
profile = "window_plus_summary"
sliding_window = 48

[conversation]
compact_enabled = true
hybrid_lane_enabled = true
```

### Telegram Bot with ACP

```toml
[provider]
kind = "openai"
model = "gpt-4o-mini"
api_key = "${OPENAI_API_KEY}"

[telegram]
enabled = true
bot_token_env = "TELEGRAM_BOT_TOKEN"
allowed_chat_ids = [123456789, -1001234567890]

[acp]
enabled = true
backend = "acpx"
default_agent = "codex"
allowed_agents = ["codex"]

[acp.dispatch]
enabled = true
conversation_routing = "all"
allowed_channels = ["telegram"]
```

---

## Validation

Validate your configuration:

```bash
# Validate default config
loongclaw validate-config

# Validate specific config
loongclaw validate-config --config /path/to/config.toml

# JSON output for programmatic use
loongclaw validate-config --json

# Fail on warnings
loongclaw validate-config --fail-on-diagnostics
```

### Common Validation Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `config.env_pointer.dollar_prefix` | Used `$VAR` instead of `${VAR}` or `VAR` | Remove `$` prefix from env pointer |
| `config.numeric_range` | Value outside allowed range | Adjust to valid range |
| `config.channel_account.duplicate_id` | Duplicate normalized account ID | Use unique account identifiers |
| `config.unknown_search_provider` | Invalid web search provider | Use `duckduckgo` (or `ddg`), `brave`, or `tavily` |

## Environment Variable References

For sensitive values, use environment variable references:

```toml
# Preferred forms
api_key = "${OPENAI_API_KEY}"
api_key_env = "OPENAI_API_KEY"

# Avoid (validation warning)
api_key_env = "$OPENAI_API_KEY"  # Don't use $ prefix
```

Common environment variables:

| Variable | Used For |
|----------|----------|
| `OPENAI_API_KEY` | OpenAI provider |
| `MOONSHOT_API_KEY` | Kimi provider |
| `KIMI_CODING_API_KEY` | Kimi Coding provider |
| `DEEPSEEK_API_KEY` | DeepSeek provider |
| `ANTHROPIC_API_KEY` | Anthropic provider |
| `BRAVE_API_KEY` | Brave Search |
| `TAVILY_API_KEY` | Tavily Search |
| `TELEGRAM_BOT_TOKEN` | Telegram bot |
| `FEISHU_APP_ID` / `FEISHU_APP_SECRET` | Feishu bot |
| `MATRIX_ACCESS_TOKEN` | Matrix bot |
