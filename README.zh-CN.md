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

Loong 是一套基于 Rust 构建的垂域智能体基建，设计目标是安全、高性能、
可扩展与可持续演进。我们希望让智能体能在真实环境中长期、可靠地工作，并与开发者
长期协作，而不只以
完成一次演示为目标。

## 为什么选择 Loong

### 安全不只是沙箱

敏感操作应在发生之前得到授权。Loong 希望让权限决策成为正常执行流程的一部分，
而不是依赖代码中散落检查，或把沙箱当作唯一安全保障。

沙箱仍可为不可信扩展提供额外保护。它是对授权的补充，而不是替代。

### 高性能从基础开始

Loong 使用 Rust 构建，目标是让核心开发保持快速，并让额外开销可预期。
当 Loong 能承担代表性的真实工作负载，将会有基准测试支撑 Loong 的具体性能结论。

### 按自己的节奏扩展

Loong 希望支持两种扩展方式：

- 需要深度集成或高性能时，使用 Rust 编写并和 Loong 一起编译。
- 需要快速迭代时，在运行期加载脚本等扩展，无需重新构建宿主程序。

两种方式都适用同样的安全要求。具体的运行期扩展系统仍在设计中。

### 面向长期演进

这次重写刻意从小而稳的基础开始。只有当职责与安全边界明晰时，
新的产品领域才会被加入。这能让 Loong 在成长过程中仍然易于理解。

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
