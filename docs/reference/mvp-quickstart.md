# MVP Quickstart

This guide covers the current MVP foundation flow for local usage.

## 0) One-Command Install

macOS/Linux:

```bash
./scripts/install.sh --setup
```

PowerShell:

```powershell
pwsh ./scripts/install.ps1 -Setup
```

## 1) Bootstrap

Generate config and local memory database:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- setup --force
```

Custom config path:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- setup --output ~/.loongclaw/config.toml --force
```

## 2) Start CLI Chat Channel

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- chat
```

Custom session:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- chat --session demo
```

Custom config:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- chat --config ~/.loongclaw/config.toml
```

Fetch latest callable model list from configured provider:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- list-models --config ~/.loongclaw/config.toml
```

JSON output:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- list-models --config ~/.loongclaw/config.toml --json
```

## 3) Start Telegram Channel

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- telegram-serve --config ~/.loongclaw/config.toml
```

Run only one polling cycle (smoke check):

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- telegram-serve --once
```

## 4) Start Feishu Channel (Webhook)

Run webhook server (default bind `127.0.0.1:8080`, path `/feishu/events`):

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- feishu-serve --config ~/.loongclaw/config.toml
```

Override bind/path at runtime:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- feishu-serve --bind 0.0.0.0:18080 --path /bot/events
```

Send proactive Feishu message:

```bash
cargo run -p loongclaw-daemon --bin loongclawd -- feishu-send --receive-id <chat_id> --text "hello"
```

## 5) Config Schema (TOML)

```toml
[provider]
kind = "openai" # anthropic/kimi/minimax/ollama/openai/openrouter/volcengine/xai/zai/zhipu
model = "auto" # auto = fetch latest available model list and select by preference
# Optional overrides; keep defaults to use kind-specific preset endpoint.
base_url = "https://api.openai.com"
chat_completions_path = "/v1/chat/completions"
# Optional override for model listing endpoint.
# models_endpoint = "https://api.openai.com/v1/models"
# API key auth (default path)
api_key_env = "OPENAI_API_KEY"
# OAuth bearer auth (takes precedence when present)
# oauth_access_token_env = "OPENAI_CODEX_OAUTH_TOKEN"
# Optional model preferences when model="auto".
# preferred_models = ["<model-id-1>", "<model-id-2>", "<model-id-3>"]
# Optional reasoning effort for providers that support reasoning controls.
# reasoning_effort = "medium" # none | minimal | low | medium | high | xhigh
temperature = 0.2
request_timeout_ms = 30000
retry_max_attempts = 3
retry_initial_backoff_ms = 300
retry_max_backoff_ms = 3000

[provider.headers]
# Optional custom headers
# "X-Request-Source" = "loongclaw"

[cli]
enabled = true
system_prompt = "You are LoongClaw, a practical assistant."
exit_commands = ["/exit", "/quit"]

[telegram]
enabled = false
bot_token_env = "TELEGRAM_BOT_TOKEN"
base_url = "https://api.telegram.org"
polling_timeout_s = 15
allowed_chat_ids = []

[feishu]
enabled = false
app_id_env = "FEISHU_APP_ID"
app_secret_env = "FEISHU_APP_SECRET"
verification_token_env = "FEISHU_VERIFICATION_TOKEN"
encrypt_key_env = "FEISHU_ENCRYPT_KEY"
base_url = "https://open.feishu.cn"
receive_id_type = "chat_id"
webhook_bind = "127.0.0.1:8080"
webhook_path = "/feishu/events"
allowed_chat_ids = []
ignore_bot_messages = true

[tools]
shell_allowlist = ["echo", "cat", "ls", "pwd"]
file_root = "."

[memory]
sqlite_path = "~/.loongclaw/memory.sqlite3"
sliding_window = 12
```

### Provider Presets (Alphabetical)

All presets use OpenAI-compatible chat completion payload shape with
provider-specific default endpoint + default API key env:

| kind | default endpoint | default key env |
| --- | --- | --- |
| `anthropic` | `https://api.anthropic.com/v1/chat/completions` | `ANTHROPIC_API_KEY` |
| `kimi` | `https://api.moonshot.cn/v1/chat/completions` | `MOONSHOT_API_KEY` |
| `minimax` | `https://api.minimaxi.com/v1/chat/completions` | `MINIMAX_API_KEY` |
| `ollama` | `http://127.0.0.1:11434/v1/chat/completions` | _(none)_ |
| `openai` | `https://api.openai.com/v1/chat/completions` | `OPENAI_API_KEY` |
| `openrouter` | `https://openrouter.ai/api/v1/chat/completions` | `OPENROUTER_API_KEY` |
| `volcengine` | `https://ark.cn-beijing.volces.com/api/v3/chat/completions` | `ARK_API_KEY` |
| `xai` | `https://api.x.ai/v1/chat/completions` | `XAI_API_KEY` |
| `zai` | `https://api.z.ai/api/paas/v4/chat/completions` | `ZAI_API_KEY` |
| `zhipu` | `https://open.bigmodel.cn/api/paas/v4/chat/completions` | `ZHIPUAI_API_KEY` |

OAuth presets supported in the same schema:

- OpenAI Codex OAuth: `oauth_access_token_env = "OPENAI_CODEX_OAUTH_TOKEN"`
- Volcengine Coding Plan OAuth: `oauth_access_token_env = "VOLCENGINE_CODING_PLAN_OAUTH_TOKEN"`

When `model = "auto"`:

- LoongClaw fetches provider model list from `models_endpoint` (or inferred default)
- if `preferred_models` is set, first matched model is used
- otherwise first model from provider catalog order (newest-first when timestamp exists)
- if selected model rejects chat endpoint or token/reasoning params, LoongClaw auto-adapts payload and can fall back to next model candidate
- recommended flow: run `list-models --json`, then copy model IDs into `preferred_models`

Backward compatibility aliases are accepted for older configs:

- `openai_compatible` -> `openai`
- `volcengine_custom` / `volcengine_compatible` -> `volcengine`
- other `*_compatible` values are accepted and mapped to the matching plain kind

If you configure Feishu event encryption (`encrypt_key_env` or `encrypt_key`),
the webhook handler enforces request signature verification using
`X-Lark-Request-Timestamp`, `X-Lark-Request-Nonce`, and `X-Lark-Signature`.

## 6) Tool Runtime (Core)

The core tool adapter currently supports:

- `shell.exec`
- `file.read`
- `file.write`

`shell.exec` is constrained by the configured shell allowlist.
`file.read` / `file.write` are constrained by configured `file_root`.

## 7) Feature Flags

Default feature set enables MVP foundation:

- `config-toml`
- `memory-sqlite`
- `tool-shell`
- `tool-file`
- `channel-cli`
- `channel-telegram`
- `channel-feishu`
- `provider-openai`
- `provider-volcengine`

Build with no default features:

```bash
cargo build -p loongclaw-daemon --no-default-features
```

Build with selected feature set:

```bash
cargo build -p loongclaw-daemon --no-default-features --features "channel-cli,config-toml,memory-sqlite"
```
