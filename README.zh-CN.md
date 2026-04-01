# 🐉 LoongClaw - 面向垂域智能体的安全基座

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="assets/logo/loongclaw-logo-light.png" />
    <img src="assets/logo/loongclaw-logo-light.png" alt="LoongClaw" width="800" />
  </picture>
</p>
<h3 align="center"><em>“发轫于东，以会群友”</em></h3>

<p align="center">
  <strong>LoongClaw 是一套基于 Rust 构建的安全、可扩展、可持续演进的 Claw 基座。</strong><br/>
  它以助手能力为起点，但目标并不仅停留于通用助手，而是逐步成长为面向团队的垂域智能体基础层，让人与 AI 能在真实场景中持续协作、共同进化。
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
  <a href="site/index.mdx">文档总览</a> •
  <a href="site/get-started/overview.mdx">快速上手</a> •
  <a href="site/use-loongclaw/common-setups.mdx">常见路线</a> •
  <a href="site/build-on-loongclaw/overview.mdx">扩展 LoongClaw</a> •
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

<a id="what-loongclaw-is"></a>
## LoongClaw 现在到底是什么

LoongClaw 从可运行的助手路径出发，但它今天已经更接近一个可扩展的 runtime foundation，而不是只在终端里工作的玩具。

今天更准确地说，它已经具备三层公开现实：

- **可运行的操作者主路径**：`loong onboard`、`loong ask`、`loong chat`、`loong doctor` 是最短的公开成功路径。
- **操作者可见的运行时表面**：`audit`、`tasks`、`skills`、`plugins`、`channels`、`runtime-snapshot` 以及 gateway control 都已经是公开命令，而不是藏在内部的东西。
- **受治理的基座能力**：provider 选择、工具、记忆、接入面、审批、策略、审计都在显式 runtime boundary 之内。

<a id="when-loongclaw-fits"></a>
## LoongClaw 适合什么场景

- **你想先从一个能跑起来的东西开始**：它从可用的 operator path 出发，而不是要求团队从裸框架原语开始自己拼。
- **你在意公开 contract 讲真话**：本地助手流、gateway ownership、reply-loop surface 和 outbound-only delivery 会明确区分，而不是混成一个大词。
- **你需要清晰的扩展缝**：provider、tool、channel、memory、policy 都有相对明确的边界，而不是鼓励大家反复直接改内核。
- **你希望运行时能跟团队一起长**：它想支撑更长期的团队工作流，而不是停在单机 prompt loop。

README 现在只保留 landing page 该有的内容。provider 实操、channel
教程和更深的 source-level reference，已经从首页里挪出去，放到 docs
里分层展开。

## 赞助商

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
## 快速开始

LoongClaw 当前主命令是 `loong`，`loongclaw` 仍保留为兼容入口。

### 安装

Linux 或 macOS：

```bash
curl -fsSL https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.sh | bash -s -- --onboard
```

Windows PowerShell：

```powershell
$script = Join-Path $env:TEMP "loong-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/loongclaw-ai/loongclaw/dev/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```

从源码安装：

```bash
bash scripts/install.sh --source --onboard
```

```bash
cargo install --path crates/daemon
```

### 第一条成功路径

```bash
loong onboard
loong ask --message "Summarize this repository and suggest the best next step."
loong chat
loong doctor --fix
```

`onboard` 是支持的 first-run 路径。它应该把你带到可用的 provider 配置和明确的下一条命令，而不是先逼你手改原始配置。

首轮上手路径刻意保持简短。完整 provider 设置、channel 配置和操作变体，应该放在 docs，而不是继续往首页里塞。

<a id="start-paths"></a>
## 从哪里开始

| 如果你需要…… | 从这里开始 |
| --- | --- |
| 先尽快得到第一条有效结果 | `onboard`、`ask`、`chat`、`doctor` |
| 想直接跟着一条完整的 provider + channel 路径走 | [Common Setups](site/use-loongclaw/common-setups.mdx) 与其下对应的 playbook 页面 |
| 不靠猜测完成 provider / model 选择 | `onboard`、`list-models`、[Providers And Models](site/use-loongclaw/providers-and-models.mdx) 以及 [Provider Recipes](site/use-loongclaw/provider-recipes.mdx) |
| 增加交付接入面，但不把支持范围说大 | [Channels](site/use-loongclaw/channels.mdx)、[Gateway And Supervision](site/use-loongclaw/gateway-and-supervision.mdx)、[Channel Recipes](site/use-loongclaw/channel-recipes.mdx) 与完整的 [Channel Setup](docs/product-specs/channel-setup.md) 说明 |
| 理解当前 runtime surface 以及受治理的扩展边界 | [Use LoongClaw](site/use-loongclaw/overview.mdx)、[Tools And Memory](site/use-loongclaw/tools-and-memory.mdx)、[ARCHITECTURE.md](ARCHITECTURE.md)、[Contributing](CONTRIBUTING.md) |

这样 README 只停留在“应该从哪里开始看”的层级，而不再承担最新完整 surface matrix 的职责。

<a id="documentation"></a>
## 文档入口

现在的公开文档刻意分成三层：

- 这个 README 只承担 landing page 的职责
- `site/` 是 Mintlify 部署的 reader-facing docs 源码
- `docs/` 留在仓库里，承载公开 source spec 和支撑性的 reference markdown

如果你是直接在仓库里阅读，这里的 docs 链接会刻意指向已提交的 docs
源码树，这样 repo reader 能直接从与 Mintlify 部署一致的材料开始。

| 如果你想…… | 从这里开始 |
| --- | --- |
| 快速拿到第一条有效结果 | [Get Started](site/get-started/overview.mdx) |
| 不自己拼接文档，直接跟一条完整 rollout path 走 | [Common Setups](site/use-loongclaw/common-setups.mdx) |
| 直接看 provider / channel 的实操路径 | [Provider Recipes](site/use-loongclaw/provider-recipes.mdx) 与 [Channel Recipes](site/use-loongclaw/channel-recipes.mdx) |
| 理解当前操作者模型 | [Use LoongClaw](site/use-loongclaw/overview.mdx) |
| 评估架构与扩展边界 | [Build On LoongClaw](site/build-on-loongclaw/overview.mdx) |
| 查看路线、策略、可靠性与发布信息 | [Reference](site/reference/overview.mdx) |
| 直接读仓库里的 source-level public contract | [ARCHITECTURE.md](ARCHITECTURE.md)、[Channel Setup](docs/product-specs/channel-setup.md)、[Roadmap](docs/ROADMAP.md) 与 [Reliability](docs/RELIABILITY.md) |

如果你是直接在仓库里读源码文档，建议先从 [Docs Overview](site/index.mdx) 开始。它承担的是 repo reader 对应的 docs landing page，而不是再把 README 重新拉长成一份大而全的 surface matrix。

<a id="architecture"></a>
## 架构速览

LoongClaw 目前是一个 7-crate Rust workspace，依赖 DAG 明确且严格：

```text
contracts (leaf -- zero internal deps)
├── kernel --> contracts
├── protocol (independent leaf)
├── app --> contracts, kernel
├── spec --> contracts, kernel, protocol
├── bench --> contracts, kernel, spec
└── daemon (binary) --> all of the above
```

最重要的三条架构规则是：

- governance-first：policy、approval、audit 都在关键执行路径中
- additive evolution：公共 contract 应该在不破坏集成的前提下持续增长
- small core, rich seams：专用化应该通过 adapter 和 pack 完成，而不是反复改内核

完整分层模型见 [ARCHITECTURE.md](ARCHITECTURE.md) 与 [Layered Kernel Design](docs/design-docs/layered-kernel-design.md)。

## 贡献

欢迎贡献。先从 [CONTRIBUTING.md](CONTRIBUTING.md) 开始。

如果你想先看哪些方向最值得补强，可以读 [Contribution Areas](site/build-on-loongclaw/contribution-areas.mdx)。
