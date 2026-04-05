# 🐉 LoongClaw - Foundation for Vertical AI Agents

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="assets/logo/loongclaw-logo-light.png" />
    <img src="assets/logo/loongclaw-logo-light.png" alt="LoongClaw" width="800" />
  </picture>
</p>
<h3 align="center"><em>"Originated from the East, here to benefit the world"</em></h3>

<p align="center">
  <strong>LoongClaw is a secure, extensible, and evolvable claw baseline built in Rust.</strong><br/>
  It starts from assistant capabilities, but it is not meant to stop at being a general assistant. Over time, it is designed to grow into a foundation for team-facing vertical agents, where people and AI can keep collaborating and evolving together.
</p>

<p align="center">
  <a href="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml?branch=dev"><img src="https://img.shields.io/github/actions/workflow/status/loongclaw-ai/loongclaw/ci.yml?branch=dev&label=build&style=flat-square" alt="Build" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/rust-edition%202024-orange.svg?style=flat-square" alt="Rust Edition 2024" />
  <a href="https://github.com/loongclaw-ai/loongclaw/releases"><img src="https://img.shields.io/github/v/release/loongclaw-ai/loongclaw?label=version&color=yellow&include_prereleases&style=flat-square" alt="Version" /></a>
  <br/>
  <a href="https://x.com/loongclawai"><img src="https://img.shields.io/badge/Follow-loongclawai-000000?logo=x&logoColor=white&style=flat-square" alt="X" /></a>
  <a href="https://t.me/loongclaw"><img src="https://img.shields.io/badge/Telegram-loongclaw-26A5E4?logo=telegram&logoColor=white&style=flat-square" alt="Telegram" /></a>
  <a href="https://discord.gg/7kSTX9mca"><img src="https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white&style=flat-square" alt="Discord" /></a>
  <a href="https://www.reddit.com/r/LoongClaw"><img src="https://img.shields.io/badge/Reddit-r%2Floongclaw-FF4500?logo=reddit&logoColor=white&style=flat-square" alt="Reddit" /></a>
  <br/>
  <a href="https://xhslink.com/m/1dqFqF1IKDk"><img src="https://img.shields.io/badge/Xiaohongshu-follow-FF2442?logo=xiaohongshu&logoColor=white&style=flat-square" alt="Xiaohongshu" /></a>
  <a href="https://loongclaw.ai/feishu.jpg"><img src="https://img.shields.io/badge/Feishu-QR-3370FF?logo=lark&logoColor=white&style=flat-square" alt="Feishu QR" /></a>
  <a href="https://loongclaw.ai/wechat.jpg"><img src="https://img.shields.io/badge/WeChat-QR-07C160?logo=wechat&logoColor=white&style=flat-square" alt="WeChat QR" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="#why-loong">Why Loong</a> •
  <a href="#product-positioning">Positioning</a> •
  <a href="#why-teams-build-on-loongclaw">Advantages</a> •
  <a href="#contributing">Contributing</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#migrate-existing-setup">Migration</a> •
  <a href="#core-capabilities">Capabilities</a> •
  <a href="#architecture-overview">Architecture</a> •
  <a href="#documentation">Docs</a>
</p>

---

<a id="why-loong"></a>

## Why Loong

We chose **Loong** deliberately.

Loong refers to the Chinese dragon. In our context, it is less about conquest or aggression and
closer to a form of strength shaped by vitality, balance, imagination, and coexistence. That feels
much closer to the spirit we want LoongClaw to carry.

LoongClaw is not meant to stop at being another generic claw. We want it to grow with people,
teams, and real working contexts, and over time become a reliable foundation for vertical agents.
For us, Loong is not only a name. It also reflects the way we want to work: respect differences,
stay open, practice reciprocity, think long-term, and stay grounded.

We want the community around LoongClaw to carry the same feeling: less noise, less posturing, and
more cooperation around real problems. If contributors, users, and partners can trust one another
and build useful things together, that matters more to us.

## Sponsors

<p align="center">
  <a href="https://www.byteplus.com/en/activity/codingplan?utm_campaign=loongclaw&utm_content=loongclaw&utm_medium=devrel&utm_source=OWO&utm_term=loongclaw">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/volcengine/volcengine-logo-dark-en.png"/>
      <img src="assets/sponsors_logo/volcengine/volcengine-logo-light-en.png" alt="Volcengine" height="44"/>
    </picture>
  </a>
  <span>&emsp;&emsp;&emsp;</span>
  <a href="https://www.feishu.cn">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/feishu/feishu-logo-dark-en.png"/>
      <img src="assets/sponsors_logo/feishu/feishu-logo-light-en.png" alt="Feishu" height="44"/>
    </picture>
  </a>
</p>

<a id="product-positioning"></a>

## Product Positioning

<p align="center">
  <img src="assets/readme/loongclaw-positioning-map.svg" alt="LoongClaw positioning map" width="100%" />
</p>

### What LoongClaw Is Today

LoongClaw today is no longer just a thin shell around a model endpoint. It is a **Rust-built claw
baseline with explicit boundaries and room to keep taking shape**. If you only look at entry
commands like `onboard`, `ask`, or `chat`, you miss the more important story: the codebase already
contains several layers that matter to teams.

| Core capability               | What is already real                                                                                                                         | Why it matters                                                                      |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| Governance-native execution   | capability tokens, policy decisions, approval requests, and audit events already sit in critical execution paths                             | this is much closer to a team system than to a single-user demo                     |
| Explicit execution planes     | `connector`, `runtime`, `tool`, and `memory` are separate kernel planes with symmetric core / extension registration                         | vertical shaping can replace planes instead of repeatedly rewriting the kernel      |
| Separate control plane        | ACP already exists as its own control plane across backend, binding, registry, runtime, analytics, and store modules                         | future routing, collaboration, and richer agent lifecycle work have a place to live |
| Shapeable context             | the context engine already has `bootstrap`, `ingest`, `after_turn`, `compact_context`, and subagent hooks                                    | context and memory are not hardcoded into a single prompt builder                   |
| Runtime-truthful tool surface | the tool catalog carries risk classes, approval modes, and `Runtime / Planned` visibility                                                    | what users see is closer to what the system can actually do right now               |
| Migration-aware setup         | `onboard` can detect current setup, Codex config, environment, and workspace guidance; the public migration CLI is now `loong migrate`       | teams do not have to rebuild configuration and long-lived context from scratch      |
| Multi-surface delivery        | beyond CLI, Telegram, Feishu / Lark, and Matrix already exist as runtime-backed surfaces with typed config, routing, and security validation | the product already reaches beyond a local terminal-only experiment                 |

That is why we increasingly describe LoongClaw as an early foundation for vertical agents. The
governance boundary, extension boundary, and delivery boundary are already visible today.

### Our Vision

The vision goes well beyond a personal assistant.

Our vision is to make LoongClaw a **foundation for vertical agents**: more focused than a general
assistant, more controllable, and better suited for real team workflows. We want teams to build
and evolve those agents faster through low-code or zero-code workflows on top of a stable core and
explicit extension seams, instead of rebuilding the system from scratch each time.

That direction does not stop at software-only agent workflows. Over time, we also care about
hardware, robotics, and embodied intelligence as natural extensions of the same foundation. The
goal is not only to connect models to chat surfaces, but to grow a base layer that can eventually
bridge digital systems and real-world action.

<a id="why-teams-build-on-loongclaw"></a>

## Why Teams Build On LoongClaw

If you place LoongClaw against a few common AI-agent product shapes, it sits between a runnable
assistant baseline and a governed vertical-agent base. The important difference is that it starts
solving team problems earlier instead of postponing them.

### Design-Orientation Comparison

| Design orientation | Assistant-first products                      | Framework-first products                                  | LoongClaw                                                                    |
| ------------------ | --------------------------------------------- | --------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Starting point     | optimize single-user chat experience first    | offer a flexible but relatively empty builder layer first | ship a runnable baseline while bringing in team-facing boundaries early      |
| Governance         | often added through perimeter systems later   | possible, but usually requires extra integration work     | policy, approval, and audit are modeled inside critical execution paths      |
| Extension model    | often grows through plugins and scripts later | highly flexible, but each team may rebuild its own stack  | extend through planes, adapters, packs, and channels with clearer boundaries |
| Delivery surfaces  | often stop at CLI or a single chat UI         | often thin on built-in delivery surfaces                  | CLI, Telegram, Feishu / Lark, and Matrix are already real delivery surfaces  |
| Vertical evolution | can stall at being "a better assistant"       | can stall at "you can build it yourself"                  | aims to keep shaping vertical agents on top of a stable Rust base            |
| Long-term edge     | usually software-assistant-centric            | usually orchestration-centric                             | leaves room for hardware, robotics, and embodied intelligence over time      |

<p align="center">
  <img src="assets/readme/loongclaw-foundation-diagram.svg" alt="LoongClaw foundation diagram" width="100%" />
</p>

<a id="quick-start"></a>

## Quick Start

### Install Script

The install script prefers the matching GitHub Release binary, verifies its SHA256 checksum,
installs `loong` as the primary command, keeps `loongclaw` as a compatibility entrypoint, and can
drop you straight into guided onboarding.

When you pass `--onboard`, the installer now seeds onboarding with a recommended
web search default. It keeps DuckDuckGo as the general key-free fallback, and
prefers Tavily when domestic Chinese locale/network hints suggest that direct
DuckDuckGo access may be a worse default. If the shell already exposes exactly
one ready credential-backed search provider such as `PERPLEXITY_API_KEY` or
`TAVILY_API_KEY`, the installer prefers that provider before falling back to
locale and route heuristics.

On Linux x86_64, the installer now treats GNU and musl as distinct release artifacts:

- it prefers `x86_64-unknown-linux-gnu` when the host glibc satisfies the declared GNU floor
- it falls back to `x86_64-unknown-linux-musl` when glibc is too old or cannot be detected
- you can override the default with `--target-libc gnu|musl` or `LOONGCLAW_INSTALL_TARGET_LIBC`

<details>
<summary>Linux / macOS</summary>

```bash
curl -fsSL https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.sh | bash -s -- --onboard
```

```bash
curl -fsSL https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.sh | bash -s -- --target-libc musl
```

</details>

<details>
<summary>Windows (PowerShell)</summary>

```powershell
$script = Join-Path $env:TEMP "loong-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```

</details>

### Build From Source

<details>
<summary>Source install</summary>

```bash
bash scripts/install.sh --source --onboard
```

```powershell
pwsh ./scripts/install.ps1 -Source -Onboard
```

```bash
cargo install --path crates/daemon
```

</details>

### Shell Completion

`loong completions <shell>` prints a completion script to stdout. GitHub
releases also publish pre-generated completion files if you prefer to download
them instead of generating them locally.

<details>
<summary>Install shell completion</summary>

```bash
loong completions bash >> ~/.bash_completion
source ~/.bash_completion
```

```zsh
loong completions zsh > "${fpath[1]}/_loong"
```

```fish
loong completions fish > ~/.config/fish/completions/loong.fish
```

```powershell
loong completions powershell >> $PROFILE
```

```elvish
loong completions elvish >> ~/.config/elvish/rc.elv
```

</details>

### First Success Path

1. Run guided onboarding:

   ```bash
   loong onboard
   ```

2. Set the provider credential that onboarding selected:

   ```bash
   export PROVIDER_API_KEY=sk-...
   ```

   If you are using Volcengine, follow the example in the
   [Configuration](#configuration) section below.

3. Get a first answer:

   ```bash
   loong ask --message "Summarize this repository and suggest the best next step."
   ```

4. Continue in session when you need follow-up work:

   ```bash
   loong chat
   ```

Optional follow-up: if you want LoongClaw to remember advisory working preferences such as preferred name, response density, or standing boundaries for future sessions, run:

```bash
loong personalize
```

5. Repair local health issues when needed:

   ```bash
   loong doctor --fix
   ```

6. Inspect the retained audit window when you need debugging evidence:

   ```bash
   loong audit recent --limit 20
   loong audit recent --kind tool-search-evaluated --query-contains "trust:official" --trust-tier official
   loong audit summary --limit 200 --json
   loong audit discovery --limit 50 --triage-label conflict
   loong audit discovery --since-epoch-s 1700010000 --until-epoch-s 1700013600
   loong audit discovery --group-by pack
   loong audit summary --pack-id sales-intel --agent-id agent-search
   loong audit summary --group-by pack
   loong audit recent --event-id evt-123 --token-id token-abc
   loong audit token-trail --token-id token-abc
   ```

When delegated work outlives one foreground turn, use the session shell to
inspect the retained session lineage directly:

```bash
loong sessions list --session default
loong sessions status --session default delegate:child-1
loong sessions history --session default delegate:child-1
loong sessions wait --session default delegate:child-1 --timeout-ms 1000
```

Channel setup comes after the base CLI path is healthy.

### Repository Observability

LoongClaw ships a built-in developer observability lane for kernel-backed
debugging and review. The app runtime writes audit events to
`~/.loongclaw/audit/events.jsonl` by default with `[audit].mode = "fanout"`, so
policy denials, token lifecycle events, and other security-critical evidence
survive process restarts.

```bash
loong doctor --config ~/.loongclaw/config.toml
loong doctor --config ~/.loongclaw/config.toml --json
loong doctor security --config ~/.loongclaw/config.toml
loong doctor security --config ~/.loongclaw/config.toml --json
loong audit recent --config ~/.loongclaw/config.toml
loong audit recent --config ~/.loongclaw/config.toml --kind tool-search-evaluated --query-contains "trust:official" --trust-tier official
loong audit summary --config ~/.loongclaw/config.toml
loong audit discovery --config ~/.loongclaw/config.toml --query-contains "trust:official" --trust-tier official
loong audit discovery --config ~/.loongclaw/config.toml --group-by agent
loong audit summary --config ~/.loongclaw/config.toml --since-epoch-s 1700010000 --until-epoch-s 1700013600
loong audit recent --config ~/.loongclaw/config.toml --pack-id sales-intel --agent-id agent-search
loong audit summary --config ~/.loongclaw/config.toml --group-by token
loong audit recent --config ~/.loongclaw/config.toml --event-id evt-123 --token-id token-abc
loong audit token-trail --config ~/.loongclaw/config.toml --token-id token-abc
loong audit recent --config ~/.loongclaw/config.toml --json
if [ -f ~/.loongclaw/audit/events.jsonl ]; then tail -n 20 ~/.loongclaw/audit/events.jsonl; else echo "audit journal is created on first audit write"; fi
```

`doctor` now surfaces audit retention mode and journal directory readiness in
addition to the existing runtime checks. For durable modes (`fanout` or
`jsonl`), LoongClaw will create the journal directory on first write, and
`doctor --fix` can pre-create it when you want a clean preflight. Use
`audit recent` when you want the bounded last-N event window and
`audit summary` when you want a compact kind/count rollup plus last-seen
fields. Use `audit discovery` when you specifically need trust-aware tool
search triage, trust-scope rollups, and the last filtered discovery context
without composing `--kind ToolSearchEvaluated` by hand. Use `audit token-trail`
when you need one token lifecycle reconstructed as a retained timeline with
issued/denied/revoked summary fields and an explicit truncation signal when the
selected `--limit` is too small to keep the full trail in view. `audit summary`
also accepts `--group-by pack|agent|token` when you need the filtered window
collapsed into grouped rollups with per-group event-kind counts, triage counts,
and last-seen metadata. `audit discovery` now also accepts `--group-by pack|agent`
so trust-aware tool-search failures can be collapsed into per-pack or per-agent
trust/triage rollups before you jump into one filtered window or token trail.
Each grouped discovery entry also carries a ready-to-run `drill_down_command`
that replays the same retained window through `audit recent` with the group
identity and active trust-aware filters already applied. `audit recent` now
also accepts `--query-contains` and `--trust-tier`, so that handoff stays
aligned with the exact discovery slice that produced the hotspot. Grouped
discovery rows also carry a `correlated_summary_command` that broadens the same
time window and workload identity into `audit summary`, so operators can pivot
from one trust-aware hotspot to the wider audit context without rebuilding the
command. Those rows now also include a compact correlated summary preview, so
you can see whether the same workload window also contains adjacent triage like
authorization denial or provider failover before switching commands. That
preview now also emits a focused signal layer with `additional_events`,
non-discovery event/triage counts, and an `attention_hint`, so adjacent audit
degradation is emphasized instead of being buried inside the full widened
summary. That focused layer now also emits a `remediation_hint`, so grouped
discovery can point from adjacent audit symptoms directly to the next operator
action instead of only telling you what widened. It now also emits a
`correlated_remediation_command`, so the strongest adjacent signal can jump
straight into the most relevant next retained-audit view instead of stopping at
advice text.
All four commands also
accept `--since-epoch-s` and `--until-epoch-s` so retained audit review can be
bounded to a concrete epoch-second window; the bounds are inclusive and are
rendered back in both text and JSON output. They also accept `--pack-id` and
`--agent-id` so retained review can collapse to one workload or one operator
session without post-processing the raw journal. When you need an exact
incident drill-down, they also accept `--event-id` and `--token-id`; the token
filter follows typed token-bearing events like `TokenIssued`, `TokenRevoked`,
and `AuthorizationDenied` instead of relying on raw string scans. Raw `tail`
remains a fallback when you need the original JSONL lines.

When provider model probing fails before any HTTP status is returned, `doctor`
now adds a provider route probe for the active request/models host. That probe
surfaces the host and port, DNS resolution results, fake-ip-style addresses,
and a short TCP reachability check so you can separate local proxy/TUN/fake-ip
instability from true upstream unavailability.

`doctor security` complements the general health check with a security exposure
and config hygiene audit. It reports `covered`, `partial`, `exposed`, and
`unknown` findings across durable audit retention, shell execution posture,
tool file-root confinement, web-fetch egress, external-skills download posture,
secret storage hygiene, and browser automation surfaces. Use the text output
for operator review and `--json` when you want a stable machine-readable
contract for automation or support tooling.

## We Are Currently Working On

<details>
<summary><strong>1. Web UI</strong></summary>
<br>

   We are currently building the first usable local LoongClaw Web UI.

   It is an optional install surface, and the current scope includes:

   - chat
   - dashboard
   - onboarding

   The initial product mode stays same-origin and localhost-bound by default.

   That default bind policy is a security boundary for the current operating
   slice, not a statement that future gateway/service expansion is out of
   scope or the long-term architecture endpoint.

   The long-term direction is to keep Web UI attached to the same daemon-owned
   gateway/service runtime rather than creating a second assistant runtime.

   This surface is still evolving and should be understood as an active MVP rather than a fully finished product interface.

   If you would like to help us continue improving it, please switch to the `web` branch and share feedback there.

</details>


## Configuration

`loong onboard` uses `provider.api_key = { env = "..." }` to reference provider credentials, so secrets stay
outside the config file:

```toml
active_provider = "openai"

[providers.openai]
kind = "openai"
api_key = { env = "PROVIDER_API_KEY" }
```

Guided onboarding now also lets you choose the default web search backend.
Supported providers are `duckduckgo`, `brave`, `tavily`, `perplexity`, `exa`,
and `jina`. If you keep the default choice, LoongClaw uses DuckDuckGo for the
general case, or Tavily when domestic Chinese locale/network hints suggest it
is the safer first-run default. When the selected provider requires a key,
onboarding immediately asks which environment variable should back that
credential and writes the config as an env reference such as
`"${TAVILY_API_KEY}"`, instead of asking users to paste the secret inline.
Non-interactive onboarding also accepts `--web-search-provider <provider>` and
`--web-search-api-key <ENV_NAME>`. Explicit choices stay explicit: LoongClaw no
longer silently falls back to DuckDuckGo when the operator explicitly selected
a credential-backed provider.

```toml
[tools.web_search]
default_provider = "duckduckgo"
# brave_api_key = "${BRAVE_API_KEY}"
# tavily_api_key = "${TAVILY_API_KEY}"
# perplexity_api_key = "${PERPLEXITY_API_KEY}"
# exa_api_key = "${EXA_API_KEY}"
# jina_api_key = "${JINA_API_KEY}"
# or "${JINA_AUTH_TOKEN}"
```

Volcengine / ARK example:

```bash
export ARK_API_KEY=your-ark-api-key
```

```toml
active_provider = "volcengine"

[providers.volcengine]
kind = "volcengine"
model = "your-coding-plan-model-id"
api_key = { env = "ARK_API_KEY" }
base_url = "https://ark.cn-beijing.volces.com"
chat_completions_path = "/api/v3/chat/completions"
```

Both `volcengine` and `volcengine_coding` use `api_key = { env = "ARK_API_KEY" }`. LoongClaw resolves that environment variable and sends it as `Authorization: Bearer <ARK_API_KEY>` on the OpenAI-compatible Volcengine path; AK/SK request signing is not used there.

Feishu channel example (webhook mode):

```bash
export FEISHU_APP_ID=cli_your_app_id
export FEISHU_APP_SECRET=your_app_secret
export FEISHU_VERIFICATION_TOKEN=your_verification_token
export FEISHU_ENCRYPT_KEY=your_encrypt_key
```

```toml
[feishu]
enabled = true
receive_id_type = "chat_id"
webhook_bind = "127.0.0.1:8080"
webhook_path = "/feishu/events"
allowed_chat_ids = ["oc_your_chat_id"]
```

```bash
loong feishu-serve --config ~/.loongclaw/config.toml
```

LoongClaw defaults to `mode = "webhook"` and reads `FEISHU_APP_ID`, `FEISHU_APP_SECRET`, `FEISHU_VERIFICATION_TOKEN`, and `FEISHU_ENCRYPT_KEY`.

Feishu channel example (websocket mode):

```bash
export FEISHU_APP_ID=cli_your_app_id
export FEISHU_APP_SECRET=your_app_secret
```

```toml
[feishu]
enabled = true
mode = "websocket"
receive_id_type = "chat_id"
allowed_chat_ids = ["oc_your_chat_id"]
```

```bash
loong feishu-serve --config ~/.loongclaw/config.toml
```

Webhook secrets are not required in websocket mode. If you are targeting Lark instead of Feishu, add `domain = "lark"`.

Assistant replies sent through `loong feishu-serve` use Feishu markdown cards when the reply fits the platform card payload limit, so Markdown renders natively in chat; oversized replies automatically fall back to plain text.

Matrix channel example:

```bash
export MATRIX_ACCESS_TOKEN=your_matrix_access_token
```

```toml
[matrix]
enabled = true
user_id = "@ops-bot:example.org"
base_url = "https://matrix.example.org"
allowed_room_ids = ["!ops:example.org"]
```

```bash
loong matrix-serve --config ~/.loongclaw/config.toml --once
```

By default, LoongClaw reads `MATRIX_ACCESS_TOKEN`. Matrix room and user IDs often contain `:`, so the runtime preserves structured Matrix route/session IDs without relying on Matrix-specific path hacks.

### Multi-Channel Serve

Use `gateway run` when you want LoongClaw to claim the explicit gateway owner
slot and supervise the enabled runtime-backed service-channel subset.

The current gateway slice now includes:

- `loongclaw gateway run` for the owner lifecycle
- `loongclaw gateway status` for cross-process owner inspection
- `loongclaw gateway stop` for cooperative shutdown
- a localhost-only authenticated control surface that publishes status,
  channel inventory, runtime snapshot, operator summary, and cooperative stop
  endpoints from the same `gateway run` owner

`gateway run` starts headless by default. Pass `--session` when you want the
concurrent CLI host attached to the same runtime owner.

When `gateway run` is active, it also binds a loopback-only control endpoint on
an ephemeral localhost port and writes the actual `bind_address`, `port`, and
`token_path` into `loongclaw gateway status --json`. Local clients such as the
future Web UI can discover the running owner through that persisted state
without introducing a second service lifecycle.

The daemon now also carries a reusable localhost gateway client/discovery layer
that centralizes loopback validation, bearer-token loading, and route helpers
for `status`, `channels`, `runtime-snapshot`, `operator-summary`, and `stop`.
That keeps dashboard and Web UI bootstrap logic out of ad-hoc file reads.

```bash
loongclaw gateway run --config ~/.loongclaw/config.toml
```

```bash
loongclaw gateway status --json
```

```bash
loongclaw gateway stop
```

`multi-channel-serve` still works as the attached compatibility wrapper when
you want one process to keep an interactive CLI session in the foreground while
supervising every enabled runtime-backed service channel in the same runtime.
It now rides on the same gateway owner contract rather than remaining the
long-term product noun.

```bash
loong multi-channel-serve \
  --session cli-supervisor \
  --channel-account telegram=bot_123456 \
  --channel-account lark=alerts \
  --channel-account matrix=bridge-sync \
  --channel-account wecom=robot-prod \
  --config ~/.loongclaw/config.toml
```

`--session` is required. Repeat `--channel-account <CHANNEL=ACCOUNT>` to pin specific channel accounts. LoongClaw normalizes runtime-backed aliases such as `lark` to canonical channel ids and only supervises runtime-backed channels that are enabled in the loaded config.

`loongclaw channels --json` exposes the broader channel catalog separately from shipped runtime-backed surfaces. That catalog now distinguishes config-backed outbound surfaces from plugin-backed bridge surfaces such as Weixin, QQBot, and OneBot. When a bridge-backed surface is configured, the same output now exposes config-derived account snapshots and bridge endpoint summaries without claiming native runtime ownership.

Plugin-backed channel surfaces are intentionally honest:

- they publish stable channel ids, setup hints, requirement keys, and target families
- they reserve native command ids such as `weixin-send` or `qqbot-serve` without claiming those commands are implemented yet
- they let bridge plugins converge on one LoongClaw-owned contract before native adapters exist

Current bridge-first target families include:

- `weixin:<account>:contact:<id>` and `weixin:<account>:room:<id>`
- `qqbot:<account>:c2c:<openid>`, `qqbot:<account>:group:<openid>`, and `qqbot:<account>:channel:<id>`
- `onebot:<account>:private:<user_id>` and `onebot:<account>:group:<group_id>`

Pure catalog-only planned surfaces such as Tlon, Nostr, Twitch, Zalo, and WebChat still do not claim runtime support until an adapter is actually shipped.

Tool policy stays explicit, and `bash.exec` is an experimental parallel tool
that does not replace `shell.exec`. Legacy `shell_default_mode`,
`shell_allow`, and `shell_deny` still feed the compatibility defaults and
compatibility rules for `bash.exec`, while the default rule files live under
`~/.loongclaw/rules`:

```toml
[tools]
shell_default_mode = "deny"
shell_allow = ["echo", "ls", "git", "cargo"]
shell_deny = ["rm"]

[tools.bash]
login_shell = false
# rules_dir = "~/.loongclaw/rules"

[tools.browser]
enabled = true
max_sessions = 8

[tools.web]
enabled = true
allowed_domains = ["docs.example.com"]
blocked_domains = ["*.internal.example"]

[tools.web_search]
enabled = true
default_provider = "duckduckgo" # or "ddg", "brave", "tavily", "perplexity", "exa", "jina"
timeout_seconds = 30
max_results = 5
# brave_api_key = "${BRAVE_API_KEY}"
# tavily_api_key = "${TAVILY_API_KEY}"
# perplexity_api_key = "${PERPLEXITY_API_KEY}"
# exa_api_key = "${EXA_API_KEY}"
# jina_api_key = "${JINA_API_KEY}"
# or "${JINA_AUTH_TOKEN}"
```

```text
# ~/.loongclaw/rules/00-allow-basic.rules
prefix_rule(pattern=["printf"], decision="allow")
prefix_rule(pattern=["git", "status"], decision="allow")

# ~/.loongclaw/rules/90-deny-dangerous.rules
prefix_rule(pattern=["rm"], decision="deny")
prefix_rule(pattern=["cargo", "publish"], decision="deny")
```

Further references:

- `default_provider` accepts `duckduckgo` (or `ddg`), `brave`, `tavily`, `perplexity` (or `perplexity_search`), `exa`, and `jina` (or `jinaai` / `jina-ai`)
- `BRAVE_API_KEY`, `TAVILY_API_KEY`, `PERPLEXITY_API_KEY`, `EXA_API_KEY`, `JINA_API_KEY`, and `JINA_AUTH_TOKEN` stay supported as environment fallbacks
- [Tool Surface Spec](docs/product-specs/tool-surface.md)
- [Product Specs](docs/product-specs/index.md)
- `loong validate-config --config ~/.loongclaw/config.toml --json`

<a id="migrate-existing-setup"></a>

## Migrate Existing Setup from Other Claws or Agents

LoongClaw does not assume teams should start from zero.

Today there are two migration-facing paths:

- `onboard` already folds current setup, Codex config, environment settings, and workspace guidance into starting-point detection, then suggests a reusable starting point.
- when you want explicit control, the public migration entrypoint is now `loong migrate`, which handles discovery, planning, selective apply, and rollback.

Its value is broader than copying a config file. LoongClaw distinguishes sources, recommends a primary source, and keeps migration split into narrower lanes such as prompt, profile, and external-skills state instead of blindly overwriting everything at once.

```bash
# Discover migration candidates under a root
loong migrate --mode discover --input ~/legacy-claws

# Plan all sources and print a recommended primary source
loong migrate --mode plan_many --input ~/legacy-claws

# Apply one selected source to a target config
loong migrate --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml --force

# Apply one selected source and bridge installable local external skills
loong migrate --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml \
  --apply-external-skills-plan --force

# Roll back the most recent migration
loong migrate --mode rollback_last_apply --output ~/.loongclaw/config.toml
```

Deeper migration modes also exist, including `merge_profiles` for multi-source profile merging and `map_external_skills` for external-skills artifact mapping. The bridge remains opt-in: prompt/profile import still works by default, while `--apply-external-skills-plan` adds installable local skill directories to the managed runtime without replacing unrelated managed skills.

<a id="manage-external-skills"></a>
## Manage External Skills

LoongClaw's external-skills runtime is operator-visible now instead of staying hidden behind migration helpers.

```bash
# Inspect resolved managed, user, and project skills with eligibility + invocation metadata
loong skills list
loong skills info release-guard

# Install one bundled first-party skill into the managed runtime
loongclaw skills install-bundled github-issues

# Inspect a bundled-only skill before installing it
loongclaw skills info native-mcp

# Inspect or install a bundled pack
loongclaw skills info anthropic-office
loongclaw skills install-bundled anthropic-office

# Download a remote skill package under the external-skills policy boundary
loong skills fetch https://skills.sh/release-guard.tgz --approve-download

# Download and sync a remote package into the managed runtime in one step
loong skills fetch https://skills.sh/release-guard.tgz \
  --approve-download --install --replace
```

`loong skills list` and `loong skills info` surface per-skill metadata such as
`invocation_policy`, required env or binaries, required runtime config gates, and declared tool
restrictions. `loong skills fetch --install --replace` gives operators a thin update path over
the existing managed install lifecycle without bypassing the same runtime policy checks that govern
downloads and installed skill execution.

Bundled first-party skills now ship with the repository and binary distribution
as local packaged assets. `loongclaw onboard` can preinstall a lightweight
subset directly during guided setup, while heavier or dependency-sensitive
bundled skills remain available through `loongclaw skills install-bundled`.

Current onboarding-exposed preinstalled skills include:

- `github-issues`
- `systematic-debugging`
- `plan`
- `Anthropic Office pack`
- `Minimax Office pack`

Current bundled-only skills that do not appear in onboarding by default include:

- `native-mcp`
- `mcporter`

The Office packs keep overlapping document skills out of the first-run choice
list while still shipping both families as bundled assets:

- `Anthropic Office pack` installs `docx`, `pdf`, `pptx`, and `xlsx`
- `Minimax Office pack` installs `minimax-docx`, `minimax-pdf`, and `minimax-xlsx`
- `skill-creator` stays separate and is not part of the Office pack grouping

Pack ids are first-class for operator workflows now:

- `loongclaw skills info anthropic-office`
- `loongclaw skills install-bundled anthropic-office`
- `loongclaw skills info minimax-office`

<a id="core-capabilities"></a>

## Core Capabilities

### Governance And Controlled Execution

- the kernel already carries governance primitives such as capability tokens, authorization,
  revocation, and audit events
- the tool catalog has built-in risk classes, approval modes, and runtime visibility, so higher-risk
  actions can move through an approval path
- browser and web tooling share the same controlled network boundary, and external skills stay
  opt-in under explicit policy

### Execution Planes And Extension Seams

- the kernel is split into four execution planes: `connector`, `runtime`, `tool`, and `memory`
- each plane supports a core / extension adapter structure, so specialization goes through explicit
  seams instead of ad-hoc kernel edits
- providers, tools, memory, channels, and packs can evolve on top of those boundaries

### Context, Memory, And Control Plane

- the context engine includes `bootstrap`, `ingest`, `after_turn`, `compact_context`, and subagent
  lifecycle hooks
- ACP acts as a separate control plane for backend, binding, registry, runtime, and related
  coordination work
- profiles, summaries, migration, canonical history, and runtime-visible recall tools such as
  `session_search`, `memory_search`, and `memory_get` together support long-lived context

### Delivery Surfaces

- CLI is first-class today, but it is no longer the only surface
- Telegram, Feishu / Lark, Matrix, and WeCom already exist as real channel surfaces with runtime state and security validation
- Weixin, QQBot, and OneBot are now first-class plugin-backed catalog surfaces, so bridge integrations can align on stable ids and target contracts without pretending LoongClaw already owns their native runtime
- browser, file, shell, and web tools are exposed through runtime policy rather than left in
  scattered helper scripts

## Architecture Overview

LoongClaw is organized as a 7-crate Rust workspace with a strict dependency DAG:

```text
contracts (leaf -- zero internal deps)
├── kernel --> contracts
├── protocol (independent leaf)
├── app --> contracts, kernel
├── spec --> contracts, kernel, protocol
├── bench --> contracts, kernel, spec
└── daemon (binary) --> all of the above
```

| Crate       | Role                                                         |
| ----------- | ------------------------------------------------------------ |
| `contracts` | Stable shared ABI surface                                    |
| `kernel`    | Policy, audit, capability, pack, and governance core         |
| `protocol`  | Typed transport and routing contracts                        |
| `app`       | Providers, tools, channels, memory, and conversation runtime |
| `spec`      | Deterministic execution specs                                |
| `bench`     | Benchmark harness and gates                                  |
| `daemon`    | Runnable CLI binary and operator-facing commands             |

Three design rules matter most:

- **governance-first**: policy, approvals, and audit are modeled in critical execution paths rather
  than bolted on later
- **additive evolution**: public contracts grow without breaking existing integrations
- **small core, rich seams**: specialization should happen through adapters and packs, not by mutating the kernel every time

### Pluggable Design, Grounded In What Exists

- **Small kernel, explicit boundaries**: `contracts`, `kernel`, `protocol`, and `app` are separated so transport, policy, runtime, and product surfaces can evolve without tangling the core.
- **Core / Extension approach**: runtime, tool, memory, and connector surfaces are organized around trusted cores with richer extension layers, so specialization goes through adapters instead of kernel forks.
- **Control planes stay distinct**: provider turns, context assembly, channel routing, and ACP control behavior are modeled as separate concerns, which keeps future collaboration and routing upgrades from forcing a rewrite of the conversation core.
- **Governance is not an afterthought**: capability checks, policy gates, approvals, and audit trails are part of the main execution path rather than a perimeter feature added later.
- **The product layer is already concrete**: a CLI-first entry path, Telegram / Feishu / Matrix / WeCom channels, plugin-backed Weixin / QQBot / OneBot catalog contracts, browser / file / shell / web tools, and configurable provider / memory / tool-policy baselines already form a real path through the current system.

Some ecosystem pieces are still better described as architecture direction than as finished product surfaces, and we prefer to say that plainly in the README.

For the full layered execution model, see [ARCHITECTURE.md](ARCHITECTURE.md) and [Layered Kernel Design](docs/design-docs/layered-kernel-design.md).

<a id="documentation"></a>

## Documentation

| Document                                                    | Description                                                                                                 |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| [Architecture](ARCHITECTURE.md)                             | Crate map and layered execution overview                                                                    |
| [Core Beliefs](docs/design-docs/core-beliefs.md)            | Core engineering principles                                                                                 |
| [SDK Docs](docs/sdk/index.md)                               | Capability authoring, internal integration seams, validator meaning, and promotion-oriented SDK references  |
| [Roadmap](docs/ROADMAP.md)                                  | Stage-based milestones and direction                                                                        |
| [Product Sense](docs/PRODUCT_SENSE.md)                      | Current product contract and user journey                                                                   |
| [Product Specs](docs/product-specs/index.md)                | User-facing requirements for onboarding, ask, doctor, channels, memory, and session recall                  |
| [Contribution Areas](docs/references/contribution-areas.md) | The kinds of design, engineering, docs, and community help that would make the biggest difference right now |
| [Reliability](docs/RELIABILITY.md)                          | Build and kernel invariants                                                                                 |
| [Security](SECURITY.md)                                     | Security policy and disclosure path                                                                         |
| [Changelog](CHANGELOG.md)                                   | Release history                                                                                             |

<a id="contributing"></a>

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full workflow.

If you want to see the areas where help is especially welcome, start with
[Contribution Areas We Especially Welcome](docs/references/contribution-areas.md).

- [Contributing Guide](CONTRIBUTING.md)
- [Contribution Areas We Especially Welcome](docs/references/contribution-areas.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## License

LoongClaw is licensed under the [MIT License](LICENSE-MIT).

Copyright (c) 2026 LoongClaw AI

## Star History

<p align="center">
  <a href="https://star-history.com/#loongclaw-ai/loongclaw&Date">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=loongclaw-ai/loongclaw&type=Date&theme=dark"/>
      <img src="https://api.star-history.com/svg?repos=loongclaw-ai/loongclaw&type=Date" alt="Star History Chart"/>
    </picture>
  </a>
</p>
