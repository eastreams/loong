# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 清理 tool descriptor/path 耦合剩余面：
   - `ToolImpl::spec()` 和 `ToolSpec` 已经不携带 path；
   - `ToolPlane::register(path, tool)` 是 path + descriptor 的组合边界；
   - `ToolPath` 已经是 app-plane-local segment path；dotted provider/catalog names 只在
     app plane 边界转换；
   - production `ReadTool` 已经不硬编码注册 path；app plane 注册时注入 user-facing 名称，
     仅用于响应和 continuation 推荐；
   - provider schema 已经用 app-owned plane enumeration gate migrated `read` / `write`
     的可见性；未注册 typed path 时不能 fallback 到 legacy static schema；
   - `read` provider JSON schema 已经来自 `ReadTool::spec().input_schema`；
   - `tool.search` schema preview 已经通过 typed provider projection 读取 `read` 的
     `ToolSpec` schema；
   - `tool.search` argument hint、search hint、tags 已经优先读取 `ToolSpec`
     discovery metadata；
   - `write` 已经随 typed app-plane 注册获得 `WriteTool::spec()` 的 provider/search
     schema、argument hint、search hint 和 tags 投影；
   - 剩余的是 catalog snapshot、governance、concurrency 等非 schema metadata 仍来自
     legacy static catalog；
   - 不把 `ToolPath` 提回 core/contracts；plane 可以继续拥有自己的 path 类型；
   - 完成线：
     - typed tools 的 agent-visible descriptor/schema 来自 plane/typed tool descriptor，而不是
       legacy static catalog；
     - legacy catalog 只描述未迁移工具，或明确标注为 legacy surface；
   - 验证：`cargo test -p loong-tools --no-default-features --features file`、
     `cargo test -p loong-app kernel_routed_file_read`、
     `cargo check -p loong-core -p loong-tools -p loong-app`、`git diff --check`。

2. 继续迁移剩余 legacy side-effect tools：
   - access/fs 已经提供 `FsWriteAction`、`FsWriteOptions` 和 `FsAccess::write_file`；
     写入 side effect 只能通过 `Granted<FsWriteAction>::run` 执行；
   - kernel 已经提供 `FsWriteAllowPolicy`，app production bootstrap 和 app test
     harnesses 已经注册该 policy；
   - `loong-tools` 已经提供 typed `WriteTool` 基础：payload parsing、tool spec
     metadata、typed output 和 access-backed execute 都在 concrete tool crate 内；
   - app plane 已经注册 typed `WriteTool`，kernel-routed `write` / `file.write`
     已走 `ctx.tool(...).invoke(...)` -> access-backed `fs.write`；
   - legacy direct `execute_tool_core_with_config` 的 `write` 分支仍在，主要承载迁移前
     runtime preview/event 语义和非 kernel-routed 调用面；
   - write/edit/config.import 按同样 access-backed action 模式迁移；
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

3. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration：
   - app bootstrap 从 config 构造 concrete policy value；
   - policy 注册使用 `PolicyPipeline::push_policy` / `push_pre_policy` /
     `push_fallback_policy`；
   - `FilePolicyExtension` 已经显式跳过 migrated `read`，避免 read 获得第二条旧授权路径；
   - tool helper、access helper、legacy direct preflight 不再读取 config 做授权；
   - 完成线：
     - filename deny、fs allowed roots、workspace root containment 都是 typed policy；
     - `FilePolicyExtension` 只剩未迁移 legacy tool 的迁移期分支，或在全部迁移后删除；
     - 配置变更通过 policy registration 改变行为，不通过 action required caps 改变行为；
   - 验证：`cargo test -p loong-kernel policy`、`cargo test -p loong-app workspace_root_tests`、
     `cargo test -p loong-app file_read`、`cargo check -p loong-app -p loong-kernel`、
     `git diff --check`。
