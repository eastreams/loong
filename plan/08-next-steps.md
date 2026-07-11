# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 接入 tool->tool 调用参数里的 capability override：
   - 当前 `ctx.tool(path)?.invoke(payload).await` 已经用 registered descriptor caps 构造
     child effective caps；
   - `ToolInvocation::invoke_with_capabilities` 已经支持 optional override，且只能缩小
     descriptor default caps；
   - 剩余的是 future tool->tool 调用入口要解析 override 参数并调用该方法；
   - 完成线：
     - tool->tool payload/descriptor 中的 override 能进入 `invoke_with_capabilities`；
     - override 扩大时返回 typed input error 或 policy denial，不能静默提升；
     - concrete tool 内部 access/action policy gate 继续读取 child effective caps；
   - 验证：`cargo test -p loong-app context::tests::`、
     `cargo test -p loong-app kernel_routed_file_read`、
     `cargo check -p loong-app -p loong-kernel`、`git diff --check`。

2. 清理 tool descriptor/path 耦合剩余面：
   - `ToolImpl::spec()` 和 `ToolSpec` 已经不携带 path；
   - `ToolPlane::register(path, tool)` 是 path + descriptor 的组合边界；
   - 剩余的是 agent prompt/catalog 仍主要从 legacy catalog 投影 path；
   - 不把 `ToolPath` 提回 core/contracts；plane 可以继续拥有自己的 path 类型；
   - 完成线：
     - typed tools 的 agent-visible path 来自 plane enumeration；
     - concrete tool crate 不出现注册路径字符串；
     - legacy catalog 只描述未迁移工具，或明确标注为 legacy surface；
   - 验证：`cargo test -p loong-tools --no-default-features --features file`、
     `cargo test -p loong-app kernel_routed_file_read`、
     `cargo check -p loong-core -p loong-tools -p loong-app`、`git diff --check`。

3. 收敛 typed invocation ingress 的 trusted overlay：
   - legacy reserved payload 字段先在 app ingress 抽成 `TrustedInvocationOverlay`；
   - typed tool 收到的 payload 不包含 reserved internal context；
   - concrete tool 不能直接读取 trusted overlay；
   - 完成线：
     - `ToolInvocation::invoke(payload)` 或其调用入口接收清理后的 agent payload；
     - trusted overlay 只影响 app orchestration，不进入 concrete tool input parse；
     - forged reserved payload 字段仍被拒绝；
   - 验证：`cargo test -p loong-app tool_invoke_rejects_forged_reserved_internal_context`、
     `cargo test -p loong-app kernel_routed_file_read`、`git diff --check`。

4. 改 `read` 为 aggregate typed tool：
   - 删除 payload-claim/fallback 思路；
   - `ReadTool` 内部解析 `path/query/pattern/glob`；
   - `path/query/glob` 分别构造不同 action；
   - `read { path, offset: 0 }` 是 typed input error，不 fallback；
   - `read { query }` / `read { pattern }` / `read { glob }` 迁入 typed path 后，旧
     direct read legacy bridge 删除；
   - 完成线：
     - app plane 注册的是 aggregate `ReadTool`，不是只接受 path payload 的 `ReadFileTool`
       fallback 机制；
     - `read { query }` / `read { pattern }` / `read { glob }` 的测试不再经过 legacy bridge；
     - file read、content search、glob path 分别有自己的 concrete action；
   - 验证：`cargo test -p loong-app direct_read`、`cargo test -p loong-app file_read`、
     `cargo test -p loong-access`、`cargo check -p loong-tools -p loong-app -p loong-access`、
     `git diff --check`。

5. 继续迁移剩余 legacy side-effect tools：
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

6. 测试清理：
   - typed tool 测试只接受 `ToolInvocation` audit；
   - legacy adapter 测试只接受 `PlaneInvoked` audit；
   - 不用 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 每个最小提交跑对应 targeted tests、`cargo check` 和 `git diff --check`；
   - 完成线：
     - `rg "PlaneInvoked.*ToolInvocation|ToolInvocation.*PlaneInvoked" crates -n` 找不到宽松断言；
     - typed path 测试名和 helper 名不再包含 legacy fallback；
     - module-level tests 留在对应模块下，例如 `tools/plane/tests.rs`、`file/tests.rs`。

7. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration：
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
