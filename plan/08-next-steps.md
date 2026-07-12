# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 继续迁移剩余 legacy side-effect tools：
   - `write` 的 context/kernel-routed 调用已走 typed plane；无 context direct `write`
     已 fail closed；
   - `edit` 和 `config.import` 仍未迁入 access-backed action 路径，且仍依赖
     `FilePolicyExtension` 的迁移期 guard；
   - `glob.search` / `content.search` 的 kernel-routed 调用已注册为 typed read-family
     path；无 context legacy adapter fallback 仍有旧 helper 入口，后续应 fail closed 或删除；
   - 逐个工具迁移：concrete tool 只解析 payload、调用 `ctx.access()` / `ctx.tool()`、
     格式化 typed output；side effect 必须落在 access crate 的 granted action run 边界；
   - 迁移完成后删除 `FilePolicyExtension` 对应旧分支；
   - 逐步清空 `Kernel::execute_tool_core` 调用面，再删除 `LegacyToolPlane` 和 adapter
     trait；
   - 完成线：
     - migrated write/edit/import 不直接调用 filesystem/network side effect；
     - side effect 只发生在 access crate 的 granted action run 边界；
     - `FilePolicyExtension` 不再覆盖已迁移工具；
   - 验证：按迁移工具分别跑对应 app/access 测试，再跑
     `cargo check -p loong-access -p loong-kernel -p loong-app -p loong` 和
     `git diff --check`。

2. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration：
   - app bootstrap 从 config 构造 concrete policy value；
   - policy 注册使用 `PolicyPipeline::push_policy` / `push_pre_policy` /
     `push_fallback_policy`；
   - tool helper、access helper、legacy direct preflight 不再读取 config 做授权；
   - 完成线：
     - filename deny、fs allowed roots、workspace root containment 都是 typed policy；
     - `FilePolicyExtension` 只剩未迁移 legacy tool 的迁移期分支，或在全部迁移后删除；
     - 配置变更通过 policy registration 改变行为，不通过 action required caps 改变行为；
   - 验证：`cargo test -p loong-kernel policy`、`cargo test -p loong-app workspace_root_tests`、
     `cargo test -p loong-app file_read`、`cargo check -p loong-app -p loong-kernel`、
     `git diff --check`。
