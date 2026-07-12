# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 继续迁移剩余 legacy side-effect tools：
   - `write` / `edit` 的 context/kernel-routed 调用已走 typed plane；无 context direct
     `write` / `edit` 已 fail closed；
   - `config.import` 仍未迁入 access-backed action 路径，且仍依赖 `FilePolicyExtension`
     的迁移期 guard；
   - 不要只把 `config.import` 入口注册进 typed plane 来假装迁移：它调用的
     `migration::*` / `config::load` / `config::write` 当前会直接读写、备份、扫描文件。
     迁移完成线必须先把这些 I/O 抽到 access-backed port 或等价的 granted action run
     边界；
   - `config.import` 的迁移先拆 filesystem primitive，再迁 tool 入口。当前 `loong-access::fs`
     已有 read/write/copy-file/glob/content-search/inspect-path/create-dir-all，仍不足以覆盖
     import 的全部副作用：
     - discovery / plan：已有受治理的 file read、directory scan、canonical/path metadata
       基础；后续迁移时仍要把调用点改成显式 I/O 边界；
     - apply：已有读取现有 output config、写 output config、创建 state dir、写 backup、
       写 import manifest、可选写 external skills manifest 的基础 primitive；后续仍要把
       `migration::*` / `config::{load,write}` 改成使用这些边界；
     - config codec：`config::parse` / `config::render` 已提供无 filesystem side effect 的
       解析/编码边界；`config::load` / `config::write` 仍是 legacy direct fs 调用点；
     - rollback：已有读取 manifest、复制 backup、恢复 output 的基础 primitive；仍缺少
       删除不存在前 output 的受治理 remove primitive；
     - apply_selected failure rollback：需要恢复 config output，并协调 skills bridge rollback。
   - 因此第一个 code 步骤不是 `Register(ConfigImportTool)`，而是把
     `migration::*` / `config::{load,write}` 依赖的 filesystem 操作改成显式 I/O 边界：
     要么新增能消费 `Granted<ConcreteFsAction>` 的 access primitives，要么让 migration
     函数接收一个 app-owned access-backed filesystem port；这个 port 不能绕过
     `ctx.access()`，也不能退回 `FilePolicyExtension`；
   - `glob.search` / `content.search` 的 kernel-routed 调用已注册为 typed read-family
     path；无 context direct 调用已 fail closed，旧 app-local search helper 已删除；
   - 逐个工具迁移：concrete tool 只解析 payload、调用 `ctx.access()` / `ctx.tool()`、
     格式化 typed output；side effect 必须落在 access crate 的 granted action run 边界；
   - 迁移完成后删除 `FilePolicyExtension` 对应旧分支；
   - 逐步清空 `Kernel::execute_tool_core` 调用面，再删除 `LegacyToolPlane` 和 adapter
     trait；
   - 完成线：
     - migrated import 不直接调用 filesystem/network side effect；
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
     - `FilePolicyExtension` 只剩 `config.import` 的迁移期分支，或在全部迁移后删除；
     - 配置变更通过 policy registration 改变行为，不通过 action required caps 改变行为；
   - 验证：`cargo test -p loong-kernel policy`、`cargo test -p loong-app workspace_root_tests`、
     `cargo test -p loong-app file_read`、`cargo check -p loong-app -p loong-kernel`、
     `git diff --check`。
