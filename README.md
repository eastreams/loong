# 🐉 LoongClaw - Rust Foundation for Vertical AI Agents

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="assets/logo/loongclaw-logo-light.png" />
    <img src="assets/logo/loongclaw-logo-light.png" alt="LoongClaw" width="800" />
  </picture>
</p>
<p align="center"><strong><em>"Originated from the East, here to benefit the world"</em></strong></p>

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

<p>
  <strong>LoongClaw is a secure, extensible, and sustainably evolvable Claw foundation built in Rust.</strong><br/>
  It starts from assistant capabilities, but its goal does not stop at being a general assistant. It is meant to grow into a team-facing foundation layer for vertical AI agents, where people and AI can keep collaborating and evolving together.
</p>

<p>
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p>
  <a href="site/index.mdx">Documentation</a> •
  <a href="site/get-started/overview.mdx">Get Started</a> •
  <a href="site/use-loongclaw/common-setups.mdx">Playbooks</a> •
  <a href="site/build-on-loongclaw/overview.mdx">Build On LoongClaw</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<a id="what-loongclaw-is"></a>
## What LoongClaw Is

LoongClaw starts from a runnable assistant path, but it is already structured as a broader runtime foundation rather than a terminal-only toy.

Today that means three concrete things:

- **🚀 Runnable operator path**: `loong onboard`, `loong ask`, `loong chat`, and `loong doctor` are the shortest supported route to a useful result.
- **👀 Operator-visible runtime surface**: `audit`, `tasks`, `skills`, `plugins`, `channels`, `runtime-snapshot`, and gateway control are public-facing commands rather than hidden internal machinery.
- **🛡️ Governed foundation**: provider selection, tools, memory, delivery surfaces, approvals, policy, and audit stay behind explicit runtime boundaries.

<a id="when-loongclaw-fits"></a>
## When LoongClaw Fits

- **⚡ You want something you can actually run first**: the project starts with a usable operator path instead of asking teams to assemble a system from framework primitives.
- **🧭 You care about truthful public contracts**: local assistant flow, gateway ownership, reply-loop surfaces, and outbound-only delivery are kept separate instead of being marketed as one thing.
- **🔌 You need visible extension seams**: providers, tools, channels, memory, and policy are explicit boundaries rather than accidental coupling points.
- **🌱 You expect the runtime to grow with the team**: the design targets longer-lived team workflows, not just a single local prompt loop.

The README is intentionally a landing page. Provider walkthroughs, channel
recipes, and deeper source-level references now live in docs instead of being
stacked into the repository front page.

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

<a id="quick-start"></a>
## Quick Start

LoongClaw uses `loong` as the primary command. `loongclaw` remains as a compatibility entrypoint.

### Install

Linux or macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.sh | bash -s -- --onboard
```

Windows PowerShell:

```powershell
$script = Join-Path $env:TEMP "loong-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```

From source:

```bash
bash scripts/install.sh --source --onboard
```

```bash
cargo install --path crates/daemon
```

### First Successful Flow

```bash
loong onboard
loong ask --message "Summarize this repository and suggest the best next step."
loong chat
loong doctor --fix
```

`onboard` is the supported first-run path. It should get you to a working provider configuration and a concrete next command without making you hand-edit raw config first.

The first-run path stays intentionally short. Full provider setup, channel configuration, and operational variants belong in docs instead of the landing page.

<a id="start-paths"></a>
## Start Paths

| If you are trying to... | Start here |
| --- | --- |
| reach first value quickly | `onboard`, `ask`, `chat`, and `doctor` |
| follow one complete provider-plus-channel rollout path | [Common Setups](site/use-loongclaw/common-setups.mdx) and the dedicated playbooks under it |
| choose a provider or model without guessing | `onboard`, `list-models`, [Providers And Models](site/use-loongclaw/providers-and-models.mdx), and [Provider Recipes](site/use-loongclaw/provider-recipes.mdx) |
| add delivery surfaces without overclaiming support | [Channels](site/use-loongclaw/channels.mdx), [Gateway And Supervision](site/use-loongclaw/gateway-and-supervision.mdx), [Channel Recipes](site/use-loongclaw/channel-recipes.mdx), and the full [Channel Setup](docs/product-specs/channel-setup.md) contract |
| understand the current runtime surface and governed extension seams | [Use LoongClaw](site/use-loongclaw/overview.mdx), [Tools And Memory](site/use-loongclaw/tools-and-memory.mdx), [ARCHITECTURE.md](ARCHITECTURE.md), and [Contributing](CONTRIBUTING.md) |

This keeps the README at the “where to begin” layer instead of turning it into the latest full command and surface matrix.

<a id="documentation"></a>
## Documentation

The public docs now work in three deliberate layers:

- this README is the landing page
- `site/` is the reader-facing docs source that Mintlify deploys
- `docs/` keeps public source specs and supporting reference markdown

When you open the repository directly, the docs links below point into the
checked-in docs source on purpose so repository readers can start from the same
material that Mintlify deploys.

| If you want to... | Start here |
| --- | --- |
| get first value quickly | [Get Started](site/get-started/overview.mdx) |
| follow one complete rollout path without stitching docs together | [Common Setups](site/use-loongclaw/common-setups.mdx) |
| follow the practical provider and channel setup paths | [Provider Recipes](site/use-loongclaw/provider-recipes.mdx) and [Channel Recipes](site/use-loongclaw/channel-recipes.mdx) |
| understand the current operator model | [Use LoongClaw](site/use-loongclaw/overview.mdx) |
| evaluate the architecture and extension seams | [Build On LoongClaw](site/build-on-loongclaw/overview.mdx) |
| check roadmap, policy, reliability, and releases | [Reference](site/reference/overview.mdx) |
| read the source-level public contracts in the repo | [ARCHITECTURE.md](ARCHITECTURE.md), [Channel Setup](docs/product-specs/channel-setup.md), [Roadmap](docs/ROADMAP.md), and [Reliability](docs/RELIABILITY.md) |

If you are reading through repository source rather than a deployed docs site, start at [Docs Overview](site/index.mdx). That page is the checked-in docs landing surface for repository readers, not another attempt to turn the README into a giant matrix.

<a id="architecture"></a>
## Architecture At A Glance

LoongClaw is organized as a 7-crate Rust workspace, but the more useful public
reading is not just "which crate depends on which." The codebase is really split
across five ownership layers: a stable contract vocabulary, a governed kernel,
a product/runtime layer, deterministic spec and benchmark rails, and a daemon
assembly layer.

```text
direct dependency DAG

contracts  (stable contract vocabulary)
├── kernel   -> contracts
├── protocol (independent transport foundation)
├── app      -> contracts, kernel
├── spec     -> contracts, kernel, protocol
├── bench    -> kernel, spec
└── daemon   -> app, bench, contracts, kernel, spec
```

In practice, those crates group into five public ownership zones:

- **Stable contracts**: `contracts` owns the shared capability, policy, audit, runtime, tool, and memory vocabulary that other crates build on.
- **Governed kernel**: `kernel` owns audit, policy, harness orchestration, runtime/tool/memory/connector planes, plugin and integration control, bootstrap, and architecture awareness.
- **Product/runtime layer**: `app` owns providers, channels, tools, memory backends, chat, conversation, session, config, and presentation surfaces.
- **Deterministic rails**: `spec` owns reproducible execution scenarios and bootstrap builders, while `bench` owns benchmark and pressure gates on top of those rails.
- **Operator assembly**: `daemon` wires the lower layers into the runnable CLI and service entrypoints such as `onboard`, `ask`, `chat`, `doctor`, `gateway`, `tasks`, `skills`, and plugin workflows.

Three design rules matter most:

- governance-first: policy, approvals, and audit stay in the real execution path
- additive evolution: public contracts should grow without breaking integrations
- small core, rich seams: specialization should happen through adapters, packs, and controlled assembly rather than repeated kernel mutation

For the full layered execution model, see [ARCHITECTURE.md](ARCHITECTURE.md) and [Layered Kernel Design](docs/design-docs/layered-kernel-design.md).
<a id="contributing"></a>
## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md).

If you want to help where it matters most right now, read [Contribution Areas](site/build-on-loongclaw/contribution-areas.mdx).
