# 🐉 Loong - 面向垂域智能体的安全基座

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/logo/loongclaw-logo-light.png" />
    <img src="./assets/logo/loongclaw-logo-light.png" alt="Loong" width="800" />
  </picture>
</p>
<p align="center"><strong><em>“发轫于东，以会群友”</em></strong></p>

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

***安全、可扩展、可持续演进***——Loong 是一套基于 Rust 构建的垂域智能体基座，在安全可控的基础上承载长程工作流构建、复合任务执行与闭环改进，让人与 AI 在真实场景中持续协作。

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="site/index.mdx">文档总览</a> •
  <a href="site/get-started/overview.mdx">快速上手</a> •
  <a href="site/use-loong/configuration-patterns.mdx">配置模式</a> •
  <a href="site/use-loong/common-setups.mdx">常见路线</a> •
  <a href="site/build-on-loong/overview.mdx">扩展 Loong</a> •
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

<a id="why-loong"></a>
## 为什么选 Loong

**因为它已经具备你在观察、操作、扩展过程中所需的核心能力：**

- **🚀 开箱即用的丰富配置**：内置 42+ provider、25+ 接入频道，几条命令即可跑通。
- **👀 透明可控的操控能力**：`audit`、`tasks`、`skills`、`plugins`、`channels`、`runtime-snapshot` 以及 gateway control 都暴露为直接可用的命令。
- **🛡️ 安全可控的基座能力**：provider 选择、工具、记忆、接入频道、审批、策略、审计都在明确的运行时边界之内。

**也因为无论你是小白还是极客，它都适合你：**

- **⚡ 易于上手**：几条命令就能跑通，兼容 OpenClaw、Claude Code、Codex、OpenCode 等同类 AI 工具的已有配置。
- **🧭 边界透明**：助手、网关、接入频道各自独立，不会混在一起。
- **🔌 内核与扩展分离**：provider、工具、接入频道、记忆、策略独立于内核，按需编译组合。
- **🌱 不是玩具**：面向长期使用设计，能跟着你的需求一起成长。

另外，如果你想读更完整的公开定位和产品立场，可以看
[Loong 的缘起与定位](site/reference/why-loong.mdx)。

## 赞助商

<p align="center">
  <a href="https://www.volcengine.com/activity/codingplan?utm_campaign=loong&utm_content=loong&utm_medium=devrel&utm_source=OWO&utm_term=loong">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/volcengine/volcengine-logo-dark-zh.png"/>
      <img src="assets/sponsors_logo/volcengine/volcengine-logo-light-zh.png" alt="火山引擎" height="44"/>
    </picture>
  </a>
  <span>&emsp;&emsp;&emsp;</span>
  <a href="https://www.feishu.cn">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/feishu/feishu-logo-dark-zh.png"/>
      <img src="assets/sponsors_logo/feishu/feishu-logo-light-zh.png" alt="飞书" height="44"/>
    </picture>
  </a>
</p>

<a id="quick-start"></a>
## 快速开始

> Loong 当前主命令是 `loong`，`loongclaw` 仍保留为兼容入口。

### 脚本安装（推荐）

Linux 或 macOS：

```bash
curl -fsSL https://raw.githubusercontent.com/eastreams/loong/dev/scripts/install.sh | bash -s -- --onboard
```

Windows PowerShell：

```powershell
$script = Join-Path $env:TEMP "loong-install.ps1"
Invoke-WebRequest https://raw.githubusercontent.com/eastreams/loong/dev/scripts/install.ps1 -OutFile $script
pwsh $script -Onboard
```

从源码安装：

确保系统有 C 链接器（Rust 编译需要）：

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

安装 Rust 工具链（已安装可跳过）：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

构建并安装：

```bash
bash scripts/install.sh --source --onboard
```

```bash
# 或者只通过 Cargo 安装（不含 onboard 引导）
cargo install --path crates/daemon
```

### 首次运行

```bash
loong onboard                # 交互式初始化，配置 provider 和 model
```

```bash
loong ask --message "用一句话总结这个仓库"  # 单轮提问，验证配置是否生效
loong chat                   # 进入多轮对话
loong doctor --fix           # 检查环境并自动修复常见问题
```

`onboard` 是支持的 first-run 路径。它应该把你带到可用的 provider 配置和明确的下一条命令，而不是先逼你手改原始配置。

首轮上手路径刻意保持简短。完整 provider 设置、channel 配置和操作变体，应该放在 docs，而不是继续往首页里塞。

当你真的需要落到原始配置时，env-backed secret 会显式写出来：

```toml
[providers.openai]
kind = "openai"
api_key = { env = "OPENAI_API_KEY" }
```

`api_key = { env = "OPENAI_API_KEY" }` 的意思是“从这个环境变量读取 secret”。`api_key = "OPENAI_API_KEY"` 则会把 `OPENAI_API_KEY` 当成字面量 key 值本身。

<a id="start-paths"></a>
## 从哪里开始

| 如果你需要…… | 从这里开始 |
| --- | --- |
| 先尽快得到第一条有效结果 | `onboard`、`ask`、`chat`、`doctor` |
| 想直接跟着一条完整的 provider + channel 路径走 | [常见路线](site/use-loong/common-setups.mdx) 与其下对应的 playbook 页面 |
| 不靠猜测完成 provider / model 选择 | `onboard`、`list-models`、[Provider 与 Model 选择](site/use-loong/providers-and-models.mdx) 以及 [Provider 路线示例](site/use-loong/provider-recipes.mdx) |
| 增加交付接入频道，但不把支持范围说大 | [接入频道选择](site/use-loong/channels.mdx)、[Gateway 与监督](site/use-loong/gateway-and-supervision.mdx)、[Channel 路线示例](site/use-loong/channel-recipes.mdx) 与完整的 [Channel Setup](docs/product-specs/channel-setup.md) 说明 |
| 理解当前 runtime surface 以及受治理的扩展边界 | [使用 Loong](site/use-loong/overview.mdx)、[工具与记忆](site/use-loong/tools-and-memory.mdx)、[ARCHITECTURE.md](ARCHITECTURE.md)、[参与贡献](CONTRIBUTING.md) |

<a id="documentation"></a>
## 文档入口

先从 `site/` 开始，它是 Mintlify 部署的 reader-facing docs 源码。`docs/`
留在仓库里，承载公开 source spec 和支撑性的 reference markdown。

如果你是直接在仓库里阅读，这里的 docs 链接会刻意指向已提交的 docs
源码树，这样 repo reader 能直接从与 Mintlify 部署一致的材料开始。

| 如果你想…… | 从这里开始 |
| --- | --- |
| 快速拿到第一条有效结果 | [快速上手](site/get-started/overview.mdx) |
| 理解项目为什么存在，以及它背后的产品立场 | [Loong 的缘起与定位](site/reference/why-loong.mdx) |
| 不自己拼接文档，直接跟一条完整 rollout path 走 | [常见路线](site/use-loong/common-setups.mdx) |
| 先理解共享的公开配置形态 | [配置模式](site/use-loong/configuration-patterns.mdx) |
| 直接看 provider / channel 的实操路径 | [Provider 指南](site/use-loong/provider-guides/index.mdx)、[Provider 路线示例](site/use-loong/provider-recipes.mdx)、[Channel 指南](site/use-loong/channel-guides/index.mdx) 与 [Channel 路线示例](site/use-loong/channel-recipes.mdx) |
| 理解当前操作者模型 | [使用 Loong](site/use-loong/overview.mdx) |
| 评估架构与扩展边界 | [扩展 Loong](site/build-on-loong/overview.mdx) |
| 查看路线、策略、可靠性与发布信息 | [参考资料](site/reference/overview.mdx) |
| 直接读仓库里的 source-level public contract | [ARCHITECTURE.md](ARCHITECTURE.md)、[Channel Setup](docs/product-specs/channel-setup.md)、[Roadmap](docs/ROADMAP.md) 与 [Reliability](docs/RELIABILITY.md) |

如果你是直接在仓库里读源码文档，建议先从 [文档总览](site/index.mdx) 开始。

<a id="architecture"></a>
## 架构速览

Loong 目前是一个 7-crate Rust workspace，但更有用的 public 读法不只是
“谁依赖谁”。按源码里的真实 ownership 来看，它其实更接近五层：稳定
contract 词汇层、受治理的 kernel、product/runtime layer、deterministic
spec/bench rails，以及 daemon assembly layer。

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

按职责看，这些 crate 可以再收敛成五个公开 ownership zone：

- **稳定 contract 层**：`contracts` 负责共享的 capability、policy、audit、runtime、tool、memory 词汇和类型。
- **受治理的 kernel 层**：`kernel` 负责 audit、policy、harness orchestration、runtime/tool/memory/connector planes、plugin 与 integration control、bootstrap、architecture awareness。
- **product/runtime layer**：`app` 负责 providers、channels、tools、memory backends、chat、conversation、session、config、presentation 等产品运行时能力。
- **deterministic rails**：`spec` 负责可复现的 execution scenarios 和 bootstrap builders，`bench` 负责构建在这些 rails 之上的 benchmark 与 pressure gates。
- **operator assembly layer**：`daemon` 把下层能力接成真正可运行的 CLI 与 service entrypoints，例如 `onboard`、`ask`、`chat`、`doctor`、`gateway`、`tasks`、`skills` 和 plugin workflows。

最重要的三条架构规则是：

- governance-first：policy、approval、audit 都在真实执行路径里
- additive evolution：公共 contract 应该在不破坏集成的前提下持续增长
- small core, rich seams：专用化应该通过 adapter、pack 和受控 assembly 完成，而不是反复改内核

完整分层模型见 [ARCHITECTURE.md](ARCHITECTURE.md) 与 [Layered Kernel Design](docs/design-docs/layered-kernel-design.md)。

## 贡献

欢迎贡献。先从 [CONTRIBUTING.md](CONTRIBUTING.md) 开始。

如果你想先看哪些方向最值得补强，可以读 [Contribution Areas](site/build-on-loong/contribution-areas.mdx)。

## Star History

<p align="center">
  <a href="https://star-history.com/#eastreams/loong&Date">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=eastreams/loong&type=Date&theme=dark"/>
      <img src="https://api.star-history.com/svg?repos=eastreams/loong&type=Date" alt="Star History Chart"/>
    </picture>
  </a>
</p>
