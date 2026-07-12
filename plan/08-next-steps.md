# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 继续迁移剩余 legacy side-effect tools：
   - `write` / `edit` 的 context/kernel-routed 调用已走 typed plane；无 context direct
     `write` / `edit` 已 fail closed；
   - `config.import` 的 kernel-routed/context-aware `plan` / `discover` / `plan_many` /
     `recommend_primary` / `merge_profiles` / `map_skills` 已通过 `ctx.access()` 读取
     import files、candidate directories、skills artifacts 和可选 output preview config；
     simple `apply` 已通过 `ctx.access()` 读取现有 output config 并写回最终 config；
     `rollback_last_apply` 已通过 `ctx.access()` 读取 manifest 并恢复/删除 output；
     `apply_selected` 在 `apply_skills_plan=false` 时已通过 `ctx.access()` 创建 state dir、
     写 backup、写 output config、原子写 import manifest；`apply_skills_plan=true`
     仍留在 legacy path，且只有这个 skills bridge 分支继续依赖 `FilePolicyExtension`
     的迁移期 guard；已迁移的 `config.import` modes 不再走 direct file preflight；
   - 不要只把 `config.import` 入口注册进 typed plane 来假装迁移：它调用的
     `migration::*` / `config::load` / `config::write` 当前会直接读写、备份、扫描文件。
     迁移完成线必须先把这些 I/O 抽到 access-backed port 或等价的 granted action run
     边界；
   - `config.import` 的迁移先拆 filesystem primitive，再迁 tool 入口。当前 `loong-access::fs`
     已有 read/write/atomic-write/copy-file/remove-file/read-dir/glob/content-search/
     inspect-path/create-dir-all，仍不足以覆盖 import 的全部副作用：
     - discovery / plan：已有受治理的 file read、一层 directory scan、canonical/path
       metadata 基础；read-only modes 的 context-aware path 已接入；
     - simple apply：已用 access 读取现有 output config、渲染 config、写最终 output
       config；不再调用 `config::{load,write}`；
     - apply_selected：无 skills bridge 的路径已迁入 access；剩余 `apply_skills_plan`
       需要把 external skills manifest、managed install/remove、失败 rollback 迁到
       新的 access/tool 调用边界；
     - config codec：`config::parse` / `config::render` 已提供无 filesystem side effect 的
       解析/编码边界；`config::load` / `config::write` 仍是 legacy direct fs 调用点；
     - rollback：kernel-routed `rollback_last_apply` 已用 access 读取 manifest、复制
       backup、恢复 output，或删除不存在前 output；remove-file 只删除文件或 symlink，且
       final component 不跟随 symlink；
     - apply_selected failure rollback：无 skills bridge 的 config output restore 已有
       access-backed path；skills bridge rollback 仍要迁移。
   - 因此下一步 code 不是 `Register(ConfigImportTool)`，而是把 `apply_selected` 的
     `apply_skills_plan=true` 路径改成显式边界：external skills install/remove 不能继续
     作为 migration 内部 direct tool side effect；external skills manifest 写入也不能退回
     direct atomic write 或 `FilePolicyExtension`。`FilePolicyExtension` 现在只标记这个
     未迁移分支，不能再扩回已迁移 modes；
   - 当前 block 在 skills lifecycle，而不是 config import payload 解析：
     `skills.install` / `skills.remove` 仍是 legacy `ToolCoreOutcome` helper，内部有
     staging/copy/rename/archive/index/remove 等 direct filesystem side effects。先迁出
     skills lifecycle 的 typed/access 边界，再让 config import 的 skills bridge 通过
     `ctx.tool(...).invoke(...)` 或等价 access-backed boundary 调用；
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
     - `FilePolicyExtension` 只覆盖 `apply_selected + apply_skills_plan=true`，并在该分支
       迁完后删除；
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
     - `FilePolicyExtension` 只剩 `config.import apply_selected + apply_skills_plan=true`
       的迁移期分支，或在该分支迁完后删除；
     - 配置变更通过 policy registration 改变行为，不通过 action required caps 改变行为；
   - 验证：`cargo test -p loong-kernel policy`、`cargo test -p loong-app workspace_root_tests`、
     `cargo test -p loong-app file_read`、`cargo check -p loong-app -p loong-kernel`、
     `git diff --check`。
