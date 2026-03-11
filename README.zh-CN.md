<!-- logo placeholder: replace with actual logo when available -->
<!-- <p align="center"><img src="logo.png" alt="LoongClaw" width="200"/></p> -->

<h1 align="center">LoongClaw</h1>

<p align="center">
  <strong>Rust 优先的 Agentic OS 基座 -- 稳定的内核协议、严格的策略边界、即插即用的运行时(runtime)扩展。</strong>
</p>

<p align="center">
  <a href="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml"><img src="https://github.com/loongclaw-ai/loongclaw/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="LICENSE-MIT"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT" /></a>
  <img src="https://img.shields.io/badge/rust-edition%202021-orange.svg" alt="Rust Edition 2021" />
  <img src="https://img.shields.io/badge/version-0.1.2--pre-yellow.svg" alt="Version: 0.1.2-pre" />
</p>

<p align="center">
  <a href="https://x.com/loongclawai"><img src="https://img.shields.io/badge/Follow-loongclawai-000000?logo=x&logoColor=white" alt="X" /></a>
  <a href="https://t.me/loongclaw"><img src="https://img.shields.io/badge/Telegram-loongclaw-26A5E4?logo=telegram&logoColor=white" alt="Telegram" /></a>
  <a href="https://discord.gg/7kSTX9mca"><img src="https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white" alt="Discord" /></a>
  <a href="https://www.reddit.com/r/LoongClaw"><img src="https://img.shields.io/badge/Reddit-r%2Floongclaw-FF4500?logo=reddit&logoColor=white" alt="Reddit" /></a>
</p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="#什么是-loongclaw">简介</a> •
  <a href="#快速开始">快速开始</a> •
  <a href="#核心功能">功能</a> •
  <a href="#架构概览">架构</a> •
  <a href="#贡献">贡献</a>
</p>

---

## 什么是 LoongClaw？

LoongClaw 是一个基于Rust构建的 Agentic OS 内核，专注于稳定且轻量的内核协议、严格的策略边界和即插即用的运行时（runtime）扩展，意在实现核心与业务功能的严格分离：

- **内核精简稳定** -- 只负责策略、安全和审计，不包含任何额外的业务逻辑，力图保持体积精简，足以在边缘设备上运行
- **安全边界不可逾越** -- 每个工具调用、内存操作和连接器调用都经过策略引擎管控；高风险操作需要显式人工授权
- **业务逻辑扩展** -- provider、工具、通道、内存后端都是可替换的适配器扩展，不侵入内核
- **多语言插件** -- 支持 Rust、WASM及任意语言的进程插件，社区可自由扩展
- **双向可集成** -- 既能作为内核被其他系统嵌入，也能通过适配器对接外部服务

## 赞助商

<p align="center">
  <a href="https://www.volcengine.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="assets/sponsors_logo/volcengine-logo-dark.png"/>
      <img src="assets/sponsors_logo/volcengine-logo-light.png" alt="火山引擎" height="48"/>
    </picture>
  </a>
  <br/><br/>
  感谢<a href="https://www.volcengine.com">火山引擎</a>对本项目的赞助支持。
</p>

## 快速开始

### 前置条件

- Rust 稳定工具链（edition 2021）
- `cargo` 在 PATH 中可用

### 从源码安装

<details>
<summary>Linux / macOS</summary>

```bash
./scripts/install.sh --setup
```
</details>

<details>
<summary>Windows (PowerShell)</summary>

```powershell
pwsh ./scripts/install.ps1 -Setup
```
</details>

<details>
<summary>手动安装 (Cargo)</summary>

```bash
cargo install --path crates/daemon
```
</details>

### 5 分钟内开始首次对话

1. 生成配置并引导本地状态：

   ```bash
   loongclaw setup
   ```

2. 设置 provider API 密钥：

   ```bash
   export PROVIDER_API_KEY=sk-...
   ```

3. 开始聊天：

   ```bash
   loongclaw chat
   ```

遇到问题请运行 `loongclaw doctor --fix`。

### 运行测试

```bash
cargo test --workspace --all-features
```

## 迁移与导入

LoongClaw 支持从旧 claw 工作区进行发现、规划、应用与回滚：

- 不传 `--mode` 时默认使用 `plan`（仅预览，不落盘）。
- `apply_selected` 同时兼容 `--source-id` 与别名 `--selection-id`。
- 安全合并同样兼容 `--primary-source-id` 与别名 `--primary-selection-id`。
- `map_external_skills` 可生成可审计、可复现的外部 skills 映射计划。
- `apply_selected` 配合 `--apply-external-skills-plan` 可把映射结果附加到 `profile_note`。
- 应用 external-skills 计划时，会额外写入 `.loongclaw-migration/<config>.external-skills.json` 便于审计与回放。

```bash
# 扫描并评分导入候选源
loongclaw import-claw --mode discover --input ~/legacy-claws

# 规划所有候选并给出推荐主源
loongclaw import-claw --mode plan_many --input ~/legacy-claws

# 预览外部 skills 映射工件与生成的 profile addendum
loongclaw import-claw --mode map_external_skills --input ~/legacy-claws

# 选择单一来源应用到目标配置
loongclaw import-claw --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml --force

# 选择来源并附加外部 skills 映射结果
loongclaw import-claw --mode apply_selected --input ~/legacy-claws \
  --source-id openclaw --output ~/.loongclaw/config.toml \
  --apply-external-skills-plan --force

# 回滚最近一次 apply/import
loongclaw import-claw --mode rollback_last_apply --output ~/.loongclaw/config.toml
```

## External Skills 运行时安全护栏

external skills 运行时采用默认安全关闭策略，必须显式开启：

- `external_skills.enabled = false`（默认禁用下载/挂载）。
- `external_skills.require_download_approval = true`（默认每次下载都需授权）。
- 域名黑名单优先级最高，命中即拒绝。
- 当 `allowed_domains` 非空时，仅允许白名单域名下载。
- `external_skills.fetch` 默认拒绝 HTTP 重定向，避免静默跨域跳转。

推荐配置基线：

```toml
[external_skills]
enabled = true
require_download_approval = true
allowed_domains = ["skills.sh", "clawhub.io"]
blocked_domains = ["*.evil.example"]
```

面向 agent 的工具：

- `external_skills_policy`
  - `action=get`：读取当前生效策略。
  - `action=set`：在运行时更新开关、授权门禁、域名白/黑名单（必须带 `policy_update_approved=true`）。
  - `action=reset`：清除运行时覆盖，回到配置文件默认值（必须带 `policy_update_approved=true`）。
- `external_skills_fetch`
  - 必填 `url`。
  - 当授权门禁开启时必须传 `approval_granted=true`。
  - 下载文件落在 `<tools.file_root>/external-skills-downloads/`。
  - 下载前强制执行白/黑名单校验。

## 核心功能

**内核与安全**
- 基于capability的策略引擎，支持令牌生命周期（发放、撤销、授权）
- 人工审批方式：逐次授权或一次性全权模式
- 插件安全扫描，`block_on_high` 强制拦截高风险操作
- WASM 静态分析（文件路径、模块大小、哈希锁定、导入策略）
- 外部配置文件完整性：校验和锁定 + ed25519 签名验证
- JSONL SIEM 导出通道，故障时可自动阻断
- 拒绝列表优先于所有授权

**运行时与执行**
- Core/Extension 适配器模式，四大模块（runtime、tool、memory、connector）均采用核心 + 扩展分层设计，扩展不可绕过核心
- 基于 Wasmtime 的 WASM 运行时执行，策略驱动的资源限制
- 进程标准 I/O 与 HTTP-JSON 桥接通道，均受协议授权保护
- 可编程的工具编排：批处理、重试、熔断器、自适应并发
- 自动发现 provider 和已扫描插件中的可用工具

**MVP 产品层**
- `setup` -- 生成 TOML 配置并引导 SQLite 内存
- `onboard` -- 引导式首次运行，带预检诊断
- `doctor` -- 诊断工具，可选安全修复 (`--fix`) 和机器可读输出 (`--json`)
- `chat` -- 交互式 CLI，滑动窗口对话记忆
- 核心工具：`shell.exec`、`file.read`、`file.write`、`external_skills.policy`、`external_skills.fetch`
- Provider：OpenAI 兼容、火山引擎自定义端点
- 通道：CLI、Telegram 轮询、飞书加密 webhook

**协议基础**
- 类型化的传输协议与方法路由
- 用于 stdio/pipe 集成的 JSON-line 流传输
- 内存通道传输，支持有界队列与背压控制
- 请求分发到处理函数前先经过授权校验

**开发者体验**
- 7 crate DAG，零循环，严格依赖方向
- 370+ 测试，每次提交都有严格的 lint/fmt CI 检查
- Cargo feature flags 支持模块化构建
- 基于 spec 的确定性测试执行
- 编程压力测试与 WASM 缓存性能的基准验收

## 架构概览

LoongClaw 组织为 7 个 crate 的工作空间，具有严格的依赖 DAG：

```text
contracts (leaf -- 零内部依赖)
├── kernel --> contracts
├── protocol (独立 leaf)
├── app --> contracts, kernel
├── spec --> contracts, kernel, protocol
├── bench --> contracts, kernel, spec
└── daemon (二进制) --> 以上全部
```

| Crate | 职责 |
|-------|------|
| `contracts` | 共享类型、能力模型。零依赖 -- 稳定的 ABI 接口。 |
| `kernel` | 策略引擎、审计事件追踪、能力令牌、插件系统、集成目录、pack 边界。 |
| `protocol` | 传输契约、类型化路由。独立 leaf crate。 |
| `app` | Provider、工具、通道、内存、配置、对话引擎。 |
| `spec` | 确定性测试场景执行器。 |
| `bench` | 基准测试框架和验收执行。 |
| `daemon` | CLI 二进制 (`loongclaw`)。将所有 crate 连接为可运行的命令。 |

完整的分层执行模型（L0-L9），请参见 [ARCHITECTURE.md](ARCHITECTURE.md)。

## Feature Flags

所有 flag 默认通过 `mvp` 启用。你可以禁用默认值，只启用所需的模块以实现最小构建。

| Flag | 描述 |
|------|------|
| `config-toml` | TOML 配置加载器 |
| `memory-sqlite` | SQLite 对话记忆 |
| `tool-shell` | `shell.exec` 工具 |
| `tool-file` | `file.read` / `file.write` 工具 |
| `channel-cli` | 交互式 CLI 通道 |
| `channel-telegram` | Telegram 轮询适配器 |
| `channel-feishu` | 飞书加密 webhook 适配器 |
| `provider-openai` | OpenAI 兼容 provider |
| `provider-volcengine` | 火山引擎自定义端点 |

最小构建示例：

```bash
cargo build -p loongclaw-daemon --no-default-features --features "channel-cli,provider-openai,config-toml,memory-sqlite"
```

## 设计原则

1. **内核优先** -- 所有执行路径都经过内核的能力/策略/审计系统。不存在绕过内核的隐藏路径。
2. **不做破坏性变更** -- 新增功能只做加法，现有公开 API 签名保持不变。
3. **默认能力管控** -- 每个操作都需要有效的 `CapabilityToken` 并匹配对应能力。
4. **审计一切安全关键操作** -- 策略拒绝、令牌生命周期事件、模块调用都发出结构化事件。
5. **7 crate DAG，零循环** -- 依赖方向不可协商。
6. **测试优先** -- 没有测试覆盖的行为，视为不存在。
7. **优先选用成熟稳定的技术** -- 选择经过验证的、可组合的依赖，而不是不透明的包。
8. **仓库是唯一真实来源** -- 如果不在仓库里，对 agent 来说它就不存在。
9. **自动化优先** -- 用 linter、CI 检查和 pre-commit hook，而非代码审查评论。
10. **严格克制过度设计** -- 当前任务所需的最小复杂度就是正确的设计方向。

## 文档

| 文档 | 描述 |
|------|------|
| [架构](ARCHITECTURE.md) | Crate 结构和分层执行模型 |
| [核心信念](docs/design-docs/core-beliefs.md) | 10 条核心工程原则 |
| [分层内核设计](docs/design-docs/layered-kernel-design.md) | 完整 L0-L9 层规格 |
| [路线图](docs/roadmap.md) | 阶段里程碑和验收标准 |
| [可靠性](docs/RELIABILITY.md) | 构建和内核不变量 |
| [示例](examples/README.md) | Spec 文件、插件示例、基准测试 |
| [产品规格](docs/product-specs/index.md) | 面向用户的需求（进行中） |
| [变更日志](CHANGELOG.md) | 发布历史 |

## 配置

`loongclaw setup` 默认通过 `provider.api_key` 引用 provider 凭据，这样密钥不会直接落在配置文件里：

```toml
[provider]
kind = "openai"
api_key = "${PROVIDER_API_KEY}"    # 推荐的显式环境变量引用写法
```

`provider.api_key` 也兼容 `$PROVIDER_API_KEY`、`env:PROVIDER_API_KEY`、`%PROVIDER_API_KEY%`，以及直接字面量写法 `api_key = "sk-..."`。
旧格式 `api_key_env = "PROVIDER_API_KEY"` 仍然兼容，但新配置建议优先使用 `provider.api_key`。

火山 Coding Plan / ARK 示例：

```toml
[provider]
kind = "volcengine"
model = "your-coding-plan-model-id"
api_key = "${ARK_API_KEY}"
base_url = "https://ark.cn-beijing.volces.com"
chat_completions_path = "/api/v3/chat/completions"
```

`kind = "volcengine"` 已经内置了上面的火山默认 endpoint，所以只有在你希望把这些值明确写进配置时，才需要额外保留 `base_url` 和 `chat_completions_path`。

验证配置：

```bash
loongclaw validate-config --config ~/.loongclaw/config.toml --json
```

## 贡献

欢迎贡献。完整的工作流请参见 [CONTRIBUTING.md](CONTRIBUTING.md)，
包括贡献流程（常规 vs. 高风险变更）和添加 provider、工具、通道的指南。

- [贡献指南](CONTRIBUTING.md)
- [行为准则](CODE_OF_CONDUCT.md)
- [安全政策](SECURITY.md)

## 许可证

LoongClaw 基于 [MIT 许可证](LICENSE-MIT) 发布。

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
