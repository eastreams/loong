# 🐉 Loong - 面向垂域智能体的安全基座

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/logo/loong-logo-dark.png" />
    <source media="(prefers-color-scheme: light)" srcset="./assets/logo/loong-logo-light.png" />
    <img src="./assets/logo/loong-logo-light.png" alt="Loong" width="280" />
  </picture>
</p>
<p align="center"><strong><em>“发轫于东，以会群友”</em></strong></p>

<p align="center">
  <a href="README.md">English</a> |
  <a href="README.zh-CN.md">简体中文</a>
</p>

> [!IMPORTANT]
> 本仓库是 Loong 的独立重写。当前只包含架构与骨架，还不是可用的
> 智能体产品。

Loong 是一套用 Rust 构建的垂域智能体基础设施。
它关注安全、性能、可扩展性与长期演进。
我们希望智能体能在真实环境中长期可靠地工作，并与开发者长期协作，
而不只完成一次演示。

## 为什么选择 Loong

### 架构治理的安全

敏感操作应在发生前获得授权。
Loong 将对外部世界的副作用建模为 `Action`，并在执行前授权。
策略授权是主要边界；沙箱和模型审查仍可作为纵深防御。

### 高性能从基础开始

Loong 使用 Rust 构建。
核心执行路径应保持快速，运行时开销应保持可预期。
具体性能结论需要代表性工作负载的基准测试支撑。

### 按自己的节奏扩展

Loong 希望支持两种扩展方式：

- 需要深度集成或高性能时，使用 Rust 编写并和 Loong 一起编译。
- 需要快速迭代时，在运行期加载脚本等扩展，无需重新构建宿主程序。

两种方式都适用同样的安全要求。具体的运行期扩展系统仍在设计中。

### 面向长期演进

这次重写刻意从小而稳的基础开始。只有当职责与安全边界明晰时，
新的产品领域才会被加入。这能让 Loong 在成长过程中仍然易于理解。

[Actor 运行时](https://github.com/InuDial/loac)及其生命周期模型让组件易于
运行、扩展、停止和重启。

## 当前状态

这次重写中，Loong 仍处于打基础的早期阶段，还不能作为完整的智能体产品安装和使用。
仓库会随着能力真正落地再如实更新说明，
因此请勿将计划中的工作当作已完成能力。

<details>
<summary>给开发者</summary>

架构记录在 [ARCHITECTURE.md](ARCHITECTURE.md)，尚未拍板的设计记录在
[docs/open-questions.md](docs/open-questions.md)。
当前的检查命令为：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
```

</details>

## 许可证

Loong 使用 [MIT License](LICENSE-MIT)。
