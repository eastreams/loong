# plan

本目录记录 Access-Action-Policy、ToolPlane、Runtime 与 recursive execution Context 的剩余迁移计划。它是
仓库内工程计划，不是 reader-facing `docs/`，也不保存已经完成的提交历史。

阅读顺序：

1. [原则与分层边界](01-principles-and-boundaries.md)：长期不变量和 owner。
2. [Runtime / Session / Context / Crate](02-runtime-and-crates.md)：recursive execution context
   形状、长期 owner 和 crate 收敛。
3. [Capability 与 Policy](03-capability-and-policy.md)：capability gate、pipeline 和 grant
   metadata。
4. [跨模块变更约束](04-change-hygiene.md)：dependency、feature、helper、comment 和 test
   hygiene。
5. [ToolPlane 与 Tool Invocation](05-tool-plane.md)：contracts-owned path、sealed dispatch 和
   legacy ingress 删除目标。
6. [Filesystem Path Grants](06-filesystem-and-read.md)：fs typestate、安全边界、operation
   ownership 和剩余 TOCTOU 工作。
7. [Kernel / Audit / 当前偏差](07-kernel-audit-and-deviations.md)：governance 与 audit
   目标，以及必须随实现同步更新的代码偏差。
8. [最小提交顺序](08-next-steps.md)：只列尚未完成的 active/deferred 目标、完成线和验证命令。

维护规则：

- 已完成步骤直接从 `08-next-steps.md` 删除；同一改动必须删除或改写
  `07-kernel-audit-and-deviations.md` 中对应的“当前偏差”，并更新 Code TODO 对照。稳定下来的
  行为只在对应原则文件中保留为不变量，不继续写成“目标形状”。
- 尚未决定的设计不能伪装成计划结论。先在讨论中决策，再写入对应文件。
- 代码中的架构迁移 `TODO(tag)` 必须映射到 `08-next-steps.md` 的一个剩余步骤。步骤完成时
  同时删除 TODO、旧分支和映射；`07` 的对应偏差或 TODO 映射仍存在时，该步骤不算完成。
- “legacy”只表示尚未迁移的 tool-core / adapter / authorization 边界。除非原则文件明确
  保留，否则 legacy 都是删除目标，不能扩展成兼容层。
