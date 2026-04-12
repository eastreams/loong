# 🐉 Loong - Rust Base for Vertical AI Agents

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/logo/loongclaw-logo-light.png" />
    <img src="./assets/logo/loongclaw-logo-light.png" alt="Loong" width="800" />
  </picture>
</p>
<p align="center"><strong><em>"Originated from the East, here to benefit the world"</em></strong></p>

<p align="center">
  <a href="https://github.com/eastreams/loong/actions/workflows/ci.yml?branch=dev"><img src="https://img.shields.io/github/actions/workflow/status/eastreams/loong/ci.yml?branch=dev&label=build&style=flat-square" alt="Build" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg?style=flat-square" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/rust-edition%202024-orange.svg?style=flat-square" alt="Rust Edition 2024" />
  <a href="https://github.com/eastreams/loong/releases"><img src="https://img.shields.io/github/v/release/eastreams/loong?label=version&color=yellow&include_prereleases&style=flat-square" alt="Version" /></a>
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

***Secure, extensible, and sustainably evolvable*** — Loong is an agent base for vertical AI agents, built in Rust. On a secure and controlled base, it supports longer-horizon workflow construction, compound task execution, and closed-loop improvement — enabling people and AI to collaborate in real-world scenarios.

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="site/index.mdx">Documentation</a> •
  <a href="site/get-started/overview.mdx">Get Started</a> •
  <a href="site/use-loong/configuration-patterns.mdx">Configuration</a> •
  <a href="site/use-loong/common-setups.mdx">Playbooks</a> •
  <a href="site/build-on-loong/overview.mdx">Build On Loong</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<a id="why-loong"></a>
## Why Loong

**Because it already has the core capabilities you need to inspect, operate, and extend:**

- **🚀 Rich configuration out of the box**: 42+ built-in providers, 25+ channels — up and running in a few commands.
- **👀 Transparent and controllable**: `audit`, `tasks`, `skills`, `plugins`, `channels`, `runtime-snapshot`, and gateway control are all exposed as directly usable commands.
- **🛡️ Secure and controllable base**: provider selection, tools, memory, channels, approvals, policy, and audit operate within explicit runtime boundaries.

**Also because whether you are a beginner or a power user, it fits you:**

- **⚡ Easy to start**: a few commands to get running, compatible with existing configurations from OpenClaw, Claude Code, Codex, OpenCode, and other similar AI tools.
- **🧭 Transparent boundaries**: assistant, gateway, and channels operate independently — never tangled together.
- **🔌 Core and extensions are separate**: providers, tools, channels, memory, and policy live outside the kernel — compile and compose as needed.
- **🌱 Not a toy**: designed for long-term use, grows with your needs over time.

Also, if you want the longer public rationale behind this positioning, read
[Why Loong](site/reference/why-loong.mdx).

## Sponsors

<p align="center">
  <a href="https://www.byteplus.com/en/activity/codingplan?utm_campaign=loong&utm_content=loong&utm_medium=devrel&utm_source=OWO&utm_term=loong">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="./assets/sponsors_logo/volcengine/volcengine-logo-dark-en.png"/>
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

<a id="quick-start"></a>
## Quick Start

> Loong uses `loong` as the primary command. `loongclaw` remains as a compatibility entrypoint.

### Script Install (Recommended)

Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/eastreams/loong/dev/scripts/install.sh | bash -s -- --onboard
```

Windows PowerShell:

```powershell
$script = Join-Path $env:TEMP "loong-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/eastreams/loong/dev/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```

From source:

Ensure your system has a C linker (required by Rust):

```bash
# Debian / Ubuntu
sudo apt update && sudo apt install build-essential
```

```bash
# Fedora
sudo dnf groupinstall "Development Tools"
```

```bash
# macOS
xcode-select --install
```

Install the Rust toolchain (skip if already installed):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Build and install:

```bash
bash scripts/install.sh --source --onboard
```

```bash
# Or install via Cargo only (without onboard setup)
cargo install --path crates/daemon
```

### First Successful Flow

```bash
loong onboard                # Interactive setup — configure provider and model
```

```bash
loong ask --message "Summarize this repo in one sentence."  # Single-turn query to verify config
loong chat                   # Start a multi-turn conversation
loong doctor --fix           # Check environment and auto-fix common issues
```

Running `onboard` is enough for the golden path — it writes a working config to `~/.loong/config.toml` without asking you to hand-edit TOML. The snippets below show what that file looks like on `dev` today, when you want to add another provider or wire up a channel.

#### Providers

```toml
active_provider = "openai"

[providers.openai]
kind = "openai"
api_key = { env = "OPENAI_API_KEY" }
model = "auto"

[providers.volcengine]
kind = "volcengine"
api_key = { env = "ARK_API_KEY" }
model = "auto"
```

- `active_provider` selects which lane runs; switch by editing the field or by running `loong onboard` again.
- `api_key = { env = "OPENAI_API_KEY" }` reads the secret from that environment variable. `api_key = "OPENAI_API_KEY"` would instead treat the string as the literal key value — a common pitfall.
- `model = "auto"` uses provider-side discovery; pin `model = "<id>"` when discovery is unreliable for your region or account.

#### Channels — Lark

```toml
[feishu]
enabled = true
domain = "lark"                           # use "feishu" for the China Feishu lane
mode = "websocket"
receive_id_type = "chat_id"
app_id = { env = "LARK_APP_ID" }
app_secret = { env = "LARK_APP_SECRET" }
allowed_chat_ids = ["oc_ops_room"]
```

Smoke-test before anything else:

```bash
loong doctor
loong feishu-send --receive-id "ou_example_user" --text "hello from loong"
loong feishu-serve
```

For the full provider and channel matrices, multi-account setups, and the long-running delivery model, see the [Documentation](#documentation) table below.

<a id="documentation"></a>
## Documentation

| | |
| --- | --- |
| Get started | [Get Started](site/get-started/overview.mdx), or just run `onboard` / `ask` / `chat` / `doctor` |
| Full rollout path | [Common Setups](site/use-loong/common-setups.mdx) |
| Pick a provider | [Provider Guides](site/use-loong/provider-guides/index.mdx) and [Provider Recipes](site/use-loong/provider-recipes.mdx) |
| Wire up channels | [Channel Guides](site/use-loong/channel-guides/index.mdx) and [Channel Recipes](site/use-loong/channel-recipes.mdx) |
| Long-running delivery | [Gateway And Supervision](site/use-loong/gateway-and-supervision.mdx) |
| Design stance | [Why Loong](site/reference/why-loong.mdx) |
| Architecture and extension | [Build On Loong](site/build-on-loong/overview.mdx) |
| Reference | [Reference](site/reference/overview.mdx) |

<a id="architecture"></a>
## Architecture

Loong is a 7-crate Rust workspace with a strict acyclic dependency graph,
organized around a governed kernel that separates contracts, security,
execution, and orchestration.

```text
contracts  (stable contract vocabulary)
├── kernel   -> contracts
├── protocol (independent transport foundation)
├── app      -> contracts, kernel
├── spec     -> contracts, kernel, protocol
├── bench    -> kernel, spec
└── daemon   -> app, bench, contracts, kernel, spec
```

For ownership zones, the layered execution model (L0–L9), and design
principles, see [ARCHITECTURE.md](ARCHITECTURE.md).

<a id="contributing"></a>
## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).

If you want to help where it matters most right now, read [Contribution Areas](site/build-on-loong/contribution-areas.mdx).

## Star History

<p align="center">
  <a href="https://star-history.com/#eastreams/loong&Date">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=eastreams/loong&type=Date&theme=dark"/>
      <img src="https://api.star-history.com/svg?repos=eastreams/loong&type=Date" alt="Star History Chart"/>
    </picture>
  </a>
</p>
