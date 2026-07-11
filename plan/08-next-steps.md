# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。

1. 删除 `loong_contracts::ToolOutcome`，把 typed tool output 改成 `Result<Value, E>`：
   - 先写/调整失败测试：
     - `loong-core` 的 erased invoke 返回 success payload，不返回
       `loong_contracts::ToolOutcome { status: "ok", payload }`；
     - `loong-tools` 的 read tool success output 是专门类型，不再直接等于
       `loong_contracts::ToolOutcome`；
     - app 的 `file.read` / `read { path }` 外部响应保持不变；
   - 修改 `crates/loong-core/src/tool.rs`：
     - 将 `ToolImpl::Output` 约束从 `Into<loong_contracts::ToolOutcome>` 收窄到 typed
       success payload，首选 `Into<serde_json::Value>`，除非实现时发现需要一个极小的
       本地 trait；
     - `RegisteredTool::invoke` 只做 sealed type erasure 和 concrete tool 调用，返回
       `Result<serde_json::Value, ToolExecutionError>`；不要在 core/erased 层拼 `"ok"`
       status；
     - 注释说明 legacy envelope 是 `ToolCoreOutcome`，不是 erased tool contract；
       concrete tool 的成功和失败必须由 `Result<Output, ToolExecutionError>` 表达；
   - 修改 `crates/tools/src/file.rs`：
     - 增加 `ReadFileOutput` 专门 struct，字段承载 read 响应所需 payload 数据；
     - `impl From<ReadFileOutput> for serde_json::Value`，把旧 payload JSON 构造搬到
       `ReadFileOutput` 的转换实现；
     - `ReadFileTool::Output = ReadFileOutput`；
     - `build_outcome` 改名为 `build_output`，返回 `Result<ReadFileOutput, String>` 或
       直接返回 `Result<ReadFileOutput, ToolExecutionError>`；
     - 不在 concrete tool 内构造 `"ok"` status；
   - `crates/contracts/src/tool_types.rs` 删除 `loong_contracts::ToolOutcome`；不要为了兼容
     concrete tool 或 erased tool 再给它加 helper；
   - `crates/app/src/tools/routing.rs` 继续只在 legacy bridge 边界把 typed success payload
     包成 `ToolCoreOutcome`，不要把 legacy envelope 泄漏进 `loong-core` 或
     `loong-tools`；
   - 完成线：
     - `rg "ToolOutcome" crates/loong-core crates/tools crates/contracts/src/tool_types.rs`
       找不到定义或 typed tool 依赖；
     - `ReadFileTool` 的 success path 返回 `ReadFileOutput`，失败 path 走 error type；
     - 只有 legacy bridge 仍能构造 `ToolCoreOutcome`；
   - 验证：`cargo test -p loong-core tool`、
     `cargo test -p loong-tools --no-default-features --features file`、
     `cargo check -p loong-app`、
     `cargo test -p loong-app kernel_routed_file_read -- --nocapture`、
     `cargo test -p loong-app file_read -- --nocapture`、
     `cargo fmt --all -- --check`、`git diff --check`。

2. 把 `ToolInvocationAction` 收回 `loong-app::tools::plane`：
   - 先写失败测试：`loong-core` 不再需要 `ToolPath` 才能编译 tool abstraction，
     app typed tool invocation 仍然产生 app-owned tool execution audit；
   - 在 `crates/app/src/tools/plane.rs` 附近定义 plane-local `ToolPath` 和
     `ToolInvocationAction`；
   - `AppToolPlane` 使用自己的 `Path`，不再直接使用 contracts `ToolPath`；`ToolPlane`
     trait 不要求公开 path 类型；
   - action payload 继续携带 agent/tool 原始 `payload: Value`，grant 后 plane 再 parse
     concrete input；
   - `ToolInvocationAction` 持有 app plane 自己的 path display/registry path，不把
     concrete path type 泄漏进 contracts/core；
   - kernel grant API 只接受 concrete `ActionMeta` 和 context，返回 `ActionGrant<A>`；
     kernel 不知道 app plane path type，也不返回 `AuthorizedToolInvocation` receipt；
   - 删除 `loong-core::tool::ToolInvocationAction`；该提交不添加 core generic helper；
   - 更新注释：core 只承载 tool abstraction，`loong-app::tools::plane` owns typed
     plane，kernel 只 grant/audit action，不执行 typed tool；
   - 完成线：
     - `crates/loong-core/src/tool.rs` 不再 import `loong_contracts::ToolPath`；
     - `crates/app/src/tools/plane.rs` 不再 import contracts `ToolPath` 或 core
       `ToolInvocationAction`；
     - `Kernel::grant_tool_invocation` 被 generic action grant 取代，或至少不再接收
       path-specific action type；
     - app typed read path 通过 `ctx.tool(path)?.invoke(payload).await` 进入，内部先 grant
       invocation action，再调用 `ToolPlane::invoke`；
     - pack/token/caps/policy denial 由 generic action grant audit 记录，不进入 app tool
       execution outcome；
     - typed path tests 断言 generic action grant audit + grant 后 app-owned tool execution
       audit，不接受 legacy `PlaneInvoked` 兜底；
   - 验证：`cargo test -p loong-core tool`、`cargo test -p loong-kernel tool_invocation`、
     `cargo test -p loong-app kernel_routed_file_read`、`cargo check -p loong-core -p
     loong-kernel -p loong-app -p loong-tools -p loong`。

3. 将 `ToolPlane` 内部存储改成 slot registry + path index：
   - 根 `Cargo.toml` 增加 `slotmap = "1"` workspace dependency，`crates/app/Cargo.toml`
     使用 `slotmap.workspace = true`；
   - 在 `crates/app/src/tools/plane.rs` 定义 private `ToolSlot`，不要 re-export；
   - 将 `AppToolPlane<C>` 从 `BTreeMap<ToolPath, RegisteredTool<C>>` 改为
     `entries: slotmap::SlotMap<ToolSlot, ToolEntry<C>>` +
     `paths: BTreeMap<ToolPath, ToolSlot>`；
   - `ToolEntry<C>` 只保存 `RegisteredTool<C>` 和 `ToolRegistration`；
     `ToolRegistration` 先至少承载 provenance，后续 registration time/source 也放在
     `ToolRegistration`；
     注释说明 slot 是内部注册句柄，不是 public identity；entry 不保存 path，避免和
     `paths` index 重复；
   - `register(path, tool)` 先检查 `paths` duplicate，再 insert entry，最后写入
     `paths.insert(path, slot)`；不要允许 alias；
   - `invoke(grant, ctx)` 先从 action 取 path，经 `paths` 查 slot，再从 `entries`
     取 entry 并调用 tool；缺失 slot 返回 `ToolPlaneError::ToolNotFound(path_display)`；
   - 测试覆盖：duplicate path 不产生第二个 entry、missing path 返回 not found、
     invoke 仍消费 grant 并执行目标 tool、slot 不出现在 public audit payload；
   - 验证：`cargo test -p loong-app tools::plane`、`cargo check -p loong-app`、
     `cargo fmt --all -- --check`、`git diff --check`。

4. 实现 effective caps / child context narrowing：
   - `AppExecutionContext` 增加 explicit effective caps 字段，`PolicyContext::capabilities()`
     返回该字段，而不是每次从 token 派生；
   - 顶层 tool invocation context 由 token caps 初始化；
   - tool 调 tool 时，根据 child tool descriptor default caps 和 optional override 构造
     child effective caps；
   - override 必须是 default caps 的子集，否则 typed input error / policy deny，不能静默
     提升；
   - child context 继承 kernel/workspace/config 等 ref 字段，但 caps 字段使用缩窄后的
     集合；
   - 完成线：
     - `PolicyContext::capabilities()` 的实现读取 context 字段，而不是每次从 token 派生；
     - child context 构造函数显式接收 narrowed caps；
     - tool invocation action 的 required caps 来自 descriptor 或 override，不在 helper 中
     即时拼装；
   - 测试覆盖：override 缩窄生效、override 扩大被拒、父 context 缺 cap 时 child 不会获得
     该 cap、domain action gate 读取的是 child effective caps；
   - 验证：`cargo test -p loong-app capabilities`、
     `cargo test -p loong-kernel policy`、`cargo check -p loong-app -p loong-kernel`、
     `cargo fmt --all -- --check`、`git diff --check`。

5. 清理 tool descriptor/path 耦合：
   - `ToolImpl::spec()` 返回无 path descriptor；
   - `RegisteredTool` 只保存 descriptor/provenance/registration metadata；
   - `ToolPlane::register(path, tool)` 组合 path + descriptor；
   - `ReadFileTool::spec()` 不再硬编码 `"read"`；
   - 完成线：
     - `crates/contracts/src/tool_types.rs` 不再有 tool descriptor path 字段；
     - `crates/tools/src/file.rs` 不再出现 `"read"` / `"file.read"` 注册 path 字符串；
     - agent prompt/catalog 的 path 显示由 plane enumeration 投影出来；
   - 验证：`cargo test -p loong-tools --no-default-features --features file`、
     `cargo test -p loong-app kernel_routed_file_read`、
     `cargo check -p loong-core -p loong-tools -p loong-app`、`git diff --check`。

6. 收敛 app typed dispatch 边界：
   - 从 `execute_kernel_tool_request` 中抽出一个聚焦的 app orchestration 边界；
   - 该边界最终落到 `ctx.tool(path)?.invoke(payload).await`；
   - `ctx.tool(path)` 返回 `Result<ToolInvocation<'_>, ToolLookupError>`，只做 plane-local
     path 解析、entry lookup 和 tool visibility 判断，不做 grant、不 parse payload；
   - `ToolInvocation::invoke(payload)` 负责读取 descriptor、计算 child caps、构造 invocation
     action -> kernel grant（自动 authorization audit）-> plane `invoke` -> grant 后
     execution audit；
   - legacy reserved payload 字段在 app ingress 抽成 `TrustedInvocationOverlay` 后，从传给
     typed tool 的 payload 中删除；concrete tool 不直接读取 trusted overlay；
   - 不引入 `AuthorizedToolInvocation` receipt workaround；
   - 完成线：
     - typed read path 的直接入口是 `ctx.tool(path)?.invoke(payload).await`；
     - `execute_kernel_tool_request` 不再手写 read-specific typed grant/invoke/audit 流程；
     - legacy fallback 只包旧 adapter/core-tool 路径，不参与 typed path；
   - 验证：`cargo test -p loong-app kernel_routed_file_read`、
     `cargo test -p loong-app direct_read`、`cargo check -p loong-app -p loong-kernel`、
     `git diff --check`。

7. 改 `read` 为 aggregate typed tool：
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

8. 继续迁移剩余 legacy side-effect tools：
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

9. 测试清理：
   - typed tool 测试只接受 `ToolInvocation` audit；
   - legacy adapter 测试只接受 `PlaneInvoked` audit；
   - 不用 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 每个最小提交跑对应 targeted tests、`cargo check` 和 `git diff --check`；
   - 完成线：
     - `rg "PlaneInvoked.*ToolInvocation|ToolInvocation.*PlaneInvoked" crates -n` 找不到宽松断言；
     - typed path 测试名和 helper 名不再包含 legacy fallback；
     - module-level tests 留在对应模块下，例如 `tools/plane/tests.rs`、`file/tests.rs`。

10. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration：
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
