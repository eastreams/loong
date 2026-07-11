# plan

本目录是 Access-Action-Policy / ToolPlane / Runtime 收敛的迁移计划入口。它仍是仓库
根目录下的迁移期工程计划，不是 reader-facing `docs/` 文档。

每个文件只承担一种阅读目的：

1. [原则与分层边界](01-principles-and-boundaries.md)：长期原则和 crate/module owner。
2. [Runtime / Context / Crate 收敛](02-runtime-and-crates.md)：二核心 runtime/context
   模型，以及哪些 crate 应保留、重定义或合并。
3. [Capability 与 Policy](03-capability-and-policy.md)：effective caps、tool->tool caps
   narrowing、config -> typed policy registration。
4. [跨模块变更约束](04-change-hygiene.md)：每个最小提交都要遵守的 dependency、feature、
   helper、comment、test hygiene。
5. [ToolPlane 与 Tool Invocation](05-tool-plane.md)：plane-local path、tool invocation
   action、sealed invocation boundary、registry shape。
6. [Filesystem Path Grants 与 `read`](06-filesystem-and-read.md)：`GrantedPath`、fs resolve/read
   action 边界、`read` 迁移状态。
7. [Kernel / Audit / 当前实现偏差](07-kernel-audit-and-deviations.md)：kernel 只做
   governance、audit 分层、截至 2026-07-11 仍存在的偏差。
8. [最小提交顺序](08-next-steps.md)：后续实现顺序、每步完成线和验证命令。

状态词约定：

- “原则”表示后续实现不能违反的约束。
- “目标形状”表示迁移完成后的结构，不代表当前代码已经满足。
- “截至 2026-07-11”表示写入计划时观察到的当前代码状态。
- “迁移期”表示只允许存在到对应工具/路径迁完；不能扩展成长期兼容层。
- “legacy”表示旧 tool-core / adapter / authorization 路径，除非明确标为保留边界，否则都是删除目标。
