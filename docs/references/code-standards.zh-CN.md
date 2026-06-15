# 代码规范

本文档定义 Loong 维护者、贡献者和 agent 协作开发需要遵守的仓库原生代码规范。

当规则比 `docs/design-docs/core-beliefs.md` 中的项目原则更具体，但又不只属于某个
crate 或某个功能时，应记录在这里。只要规则可以被机械化执行，就优先使用脚本、
lint 或 CI 门禁，而不是依赖人工 review。

## 范围

本文档覆盖：

- Rust 代码形态和可维护性预期
- 函数、模块和文件大小指导
- 测试位置和测试文件组织方式
- fixture、snapshot 和生成产物的处理方式
- 错误处理和可观测性约定
- 代码规范漂移的 review 检查项
- 当前和计划中的执行门禁

## Rust 代码形态

当项目级 Rust 风格规则超出 rustfmt 和 Clippy 默认能力时，将其记录在这里。

- 优先使用显式、狭窄的数据流，避免隐藏的全局状态。
- 除非已有文档化的破坏性变更决策，否则公共 API 保持增量演进。
- 除非相关架构文档明确允许，否则不要把领域特定行为放进更底层的 crate。
- 只有在能让调用点更清晰，或能消除有意义的重复时，才引入局部 helper 函数。
- 对于可以用标准库或现有 workspace 依赖清晰表达的小工具，避免引入新依赖。

## 函数和模块大小

本节用于记录具体的大小预算和重构触发条件。

- 函数应保持在 reviewer 可以一次性看清控制流的范围内。
- 长函数需要有明确理由，例如表驱动解析、结构化命令接线，或内联后更易读的测试
  setup。
- 当校验、转换、执行或渲染步骤可以被独立命名和测试时，应拆分函数。
- 模块应聚焦在单一职责上。如果一个文件积累了不相关的 helper 家族，优先把 helper
  移到拥有该功能或 crate surface 的位置附近。

在把机械化预算加入脚本或 CI 之前，应先在这里记录提案。

| Surface | Target | Enforcement |
| --- | --- | --- |
| Function length | Decide threshold before enforcing | Manual review for now |
| File length | Decide threshold before enforcing | Manual review for now |
| Test module length | Decide threshold before enforcing | Manual review for now |

## 测试文件组织

测试应当记录行为，而不仅仅是提高覆盖率数字。

- 当测试私有 helper 或狭窄模块行为时，单元测试应放在实现旁边。
- 集成测试应放在与被测试公共行为匹配的 crate 或 workspace 测试 surface 下。
- 测试文件应按行为或运行时 surface 分组，而不是按偶然的 bug report 历史分组。
- 优先使用确定性的输入和输出。除非测试明确用于验证对应集成，否则不应依赖真实
  home 目录、在线网络服务、墙钟时间或共享可变主机状态。
- 大型 setup helper 和 fixture 应按领域命名，让未来的 agent 和 reviewer 能找到它们
  存在的原因。

## 测试命名

测试名称应清楚说明行为和预期。

- 用测试保护的规则或场景来命名测试。
- 当行为有更精确名称时，避免使用 `test_basic`、`test_error` 或 `test_success` 这类
  含糊名称。
- 必要时可以在注释或 PR 文本中引用 regression 背景，但测试名称本身应聚焦于必须
  持续成立的行为。

## Fixtures、Snapshots 和生成产物

Fixture 和 snapshot 是被 review 的契约的一部分。

- 除非多个 crate 或测试套件有意共享，否则 fixture 应存放在拥有它们的测试附近。
- Fixture 数据应尽量精简，并且为具体目的服务。
- 不要在没有 review 其语义变化的情况下更新 snapshot 或生成产物。
- 生成文件应标明生成器；除非文件头明确允许，否则不要手动编辑。

## 错误处理和可观测性

错误路径应足够明确，让用户、操作者和 agent 不需要先阅读无关源码就能调试问题。

- 当所在 crate 已有结构化错误或类型化诊断模式时，应返回结构化错误或类型化诊断。
- 不要用泛化字符串隐藏 policy denial、capability failure 或 audit-write failure。
- 安全关键行为应按 kernel 和 reliability 文档要求产生 audit evidence。
- 生产代码中避免使用 `unwrap`、`expect`、`panic`、`todo` 和 `unimplemented`。Workspace
  Clippy 设置已经会拒绝这些模式。

## Review 检查清单

Review 对代码规范敏感的变更时，使用这份检查清单：

- 变更是否保持了 crate 依赖方向？
- 函数和模块是否仍然足够聚焦，可以在局部范围内 review？
- 新测试是否放在未来维护者会预期的位置？
- 测试名称描述的是稳定行为，而不是实现细节吗？
- Fixture、snapshot 和生成文件的变化是否是有意的？
- 错误路径是否明确且可观测？
- 是否有任何重复出现的 review 评论可以转化为脚本、lint 或 CI 检查？

## 执行门禁

当前机械化执行包括：

- 通过 `./scripts/cargo-local-toolchain.sh fmt --all -- --check` 运行 `cargo fmt`
- 通过 `./scripts/cargo-local-toolchain.sh clippy --workspace --all-targets --all-features -- -D warnings`
  运行严格 Clippy
- 通过 `./scripts/cargo-local-toolchain.sh test --workspace` 运行 workspace 测试
- 通过 `./scripts/cargo-local-toolchain.sh test --workspace --all-features` 运行 all-feature 测试
- 通过 `scripts/check_dep_graph.sh` 和 `scripts/check_architecture_boundaries.sh` 运行依赖和架构检查

未来的大小、测试布局或 fixture 检查应先记录在这里，再加入本地验证或 CI。
