# 🐉 Loong - 面向垂域智能体的安全基座

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/logo/loongclaw-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/logo/loongclaw-logo-light.png" />
    <img src="./assets/logo/loongclaw-logo-light.png" alt="Loong" width="280" />
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

走完 `onboard` 就够了 —— 它会把一份能跑的配置写到 `~/.loong/config.toml`，不需要你手写 TOML。如果你想再加一个 provider 或接入频道，下面几段是 dev 分支当前的实际形态。

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

- `active_provider` 决定当前运行的 lane，直接改字段或重跑一次 `loong onboard` 都能切换。
- `api_key = { env = "OPENAI_API_KEY" }` 表示从环境变量读取；写成 `api_key = "OPENAI_API_KEY"` 会被当成字面量密钥值，这是常见踩坑。
- `model = "auto"` 走 provider 端自动发现；如果你所在区域或账号下自动发现不稳，改成 `model = "<具体 id>"` 固定即可。

#### 接入频道 —— 以飞书为例

```toml
[feishu]
enabled = true
domain = "feishu"                         # 国际版 Lark 改成 "lark"
mode = "websocket"
receive_id_type = "chat_id"
app_id = { env = "FEISHU_APP_ID" }
app_secret = { env = "FEISHU_APP_SECRET" }
allowed_chat_ids = ["oc_ops_room"]
```

先快速验证一下：

```bash
loong doctor
loong feishu-send --receive-id "ou_example_user" --text "hello from loong"
loong feishu-serve
```

完整的 provider / channel 矩阵、多账号配置与长期托管模型，继续看下面的 [文档](#documentation) 表。

<a id="documentation"></a>
## 文档

| | |
| --- | --- |
| 快速上手 | [Get Started](site/get-started/overview.mdx)，或直接用 `onboard` / `ask` / `chat` / `doctor` |
| 完整路径 | [Common Setups](site/use-loong/common-setups.mdx) |
| 选 Provider | [Provider Guides](site/use-loong/provider-guides/index.mdx) 与 [Provider Recipes](site/use-loong/provider-recipes.mdx) |
| 接入频道 | [Channel Guides](site/use-loong/channel-guides/index.mdx) 与 [Channel Recipes](site/use-loong/channel-recipes.mdx) |
| 长期托管 | [Gateway 与监督](site/use-loong/gateway-and-supervision.mdx) |
| 设计立场 | [Why Loong](site/reference/why-loong.mdx) |
| 架构与扩展 | [Build On Loong](site/build-on-loong/overview.mdx) |
| 参考资料 | [Reference](site/reference/overview.mdx) |

<a id="architecture"></a>
## 架构

Loong 是一个 7-crate Rust workspace，依赖图严格无环，围绕一个受治理的
kernel 组织，将 contract、安全、执行、编排几个关注点分开。

```text
contracts  (stable contract vocabulary)
├── kernel   -> contracts
├── protocol (independent transport foundation)
├── app      -> contracts, kernel
├── spec     -> contracts, kernel, protocol
├── bench    -> kernel, spec
└── daemon   -> app, bench, contracts, kernel, spec
```

ownership 分区、分层执行模型（L0–L9）以及设计原则，见
[ARCHITECTURE.md](ARCHITECTURE.md)。

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
