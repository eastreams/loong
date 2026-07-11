# plan: Kernel / Audit / 当前实现偏差

本文件定义 kernel/audit 边界，并列出截至 2026-07-11 的实现偏差。后续最小提交顺序见
`08-next-steps.md`。

## Kernel

Kernel 不再提供 typed `Kernel::invoke_tool`，也不持有 typed `tool_plane` 字段。

目标 API：

```rust
Kernel::grant(
    pack_id,
    token,
    impl ActionMeta,
    &ctx,
) -> ActionGrant<A>
```

tool invocation 的治理入口不需要知道 concrete path 类型；它只需要 `ActionMeta` 提供
operation/payload/required capabilities。pack boundary、token boundary、policy pipeline
仍在 kernel 检查。

`Kernel::grant` 不做 tool dispatch，也不拥有 tool registry。grant 过程天然记录
authorization audit：action metadata、required caps、policy report、allow/deny 和
grant id 都记录到 kernel audit sink。tool invocation 的 denied evidence 因而属于 generic action
grant audit，不需要单独的 `ToolInvocationOutcome::Denied` 或 receipt workaround。

grant 后的 execution outcome audit 属于 grant consumption 边界。对 tool 来说，该边界
是 `ctx.invoke_tool(...) -> ToolPlane::invoke(...)`；对 fs 来说，是
`Granted<FsReadAction>::run(ctx)`。它只能记录 grant 已经发放之后的 completed / failed /
input error 等结果，不能重复表达 authorization deny。

旧 `Kernel::execute_tool_core` 只在迁移期服务 legacy fallback。旧 `CoreToolAdapter` /
`ToolExtensionAdapter` 只能被 `LegacyToolPlane` 包住，迁移完后删除。


## Audit

audit 分两层，不再把所有内容塞进 kernel/contract enum。

- kernel 只拥有 `AuditSink`、event id/clock/grant id，以及 generic action authorization
  audit。`Kernel::grant` 对任意 action 统一记录 action metadata、required caps、
  policy report、allow/deny 和 grant id。
- app 拥有 tool-specific execution audit schema。tool path display、tool execution
  outcome、legacy fallback 对比都属于 app runtime 语义；app 可以把这些事件写入 kernel
  提供的 sink，但 kernel 不需要定义这些业务 enum。

typed tool invocation 应该有 audit evidence。tool 调用是 agent/user 可见的治理边界；
authorization allow/deny 由 generic action grant audit 记录；grant 后的执行成功/失败由
app-owned execution audit 记录。否则 typed tool 从 legacy `PlaneInvoked` 迁走后，证据链
反而变少。

tool-specific audit event 不能固化到 contracts/kernel 的 ToolPlane registry key 类型。
此前把 `ToolInvocation { path: ToolPath, ... }` 加到 `AuditEventKind` 里是过度固化。app
层可以为 audit payload 存 `path_display`，因为 audit 需要的是可读、稳定、可关联的 path
表示，不是具体 plane 的 key。`ToolPath` 不应该成为 contracts/core 的全局类型。

app-owned tool execution event 只记录 grant 后结果：

```rust
ToolInvocation {
    pack_id,
    path_display,
    grant_id,
    execution_outcome,
}
```

`path_display` 是 app audit payload，不是 registry key 类型。不同 `ToolPlane` 可以
有不同 path model，只要 app 在 audit 中给出稳定、可读、可关联的表示。

`ToolInvocationOutcome` 如果保留，应放在 app 层，只能描述 grant 后 execution outcome，
例如 completed / failed / input_error；不能包含 denied 分支，不能隐含 fallback 机制为
kernel contract。

legacy adapter 在迁移期继续记录旧 `PlaneInvoked`，直到对应工具迁移完成。

### Tool audit failure matrix

authorization audit 由 `Kernel::grant` 强制记录，concrete `ToolImpl` 不拿 audit API。
execution audit 由 `ctx.invoke_tool` / granted action run 边界强制记录。

- invocation grant 被 pack/token/caps 拒绝：`Kernel::grant` 记录 generic action grant deny，
  plane 不执行。
- invocation policy 被 `PolicyPipeline` 拒绝：`Kernel::grant` 记录带 `PolicyReport` 的
  generic action grant deny，plane 不执行。
- payload parse / typed input error：grant 已消费进入 plane，app orchestration 记录
  grant 后 execution failed/input_error，不 fallback。
- concrete tool execution error：app orchestration 记录
  grant 后 execution failed。
- tool 内部 access/action policy denial：domain access 返回 authorization error；app
  orchestration 把本次 tool invocation 记为 failed。domain action 的 policy evidence
  保留在 `PolicyGrantError::Denied { report, ... }`，不要把它伪装成 tool route。
- legacy fallback：继续记录 `PlaneInvoked`，直到该 tool 迁入 typed plane；typed 测试不再
  接受 `PlaneInvoked | ToolInvocation` 这种宽松断言。

截至 2026-07-11，`Kernel::grant_tool_invocation` 内部记录部分 deny audit，这是迁移期 helper 行为。目标是
generic grant 负责所有 action authorization audit；tool invocation execution audit 留在
`ctx.invoke_tool` 的 grant consumption 边界。


## 当前实现偏差

以下偏差是截至 2026-07-11 的代码状态；它们描述迁移目标，不是新代码应继续复用的形状：

- `crates/contracts/src/tool_types.rs` 仍定义全局 `ToolPath`，且 `ToolSpec` 仍携带
  `path`。目标是 descriptor 无 path，注册点/plane 才绑定 path。
- `crates/loong-core/src/tool.rs` 里的 `ToolInvocationAction` 持有全局 `ToolPath`。
  `ToolInvocationAction` 属于 app/plane-owned action；迁移目标是删除 core action，不新增
  core generic helper。
- `crates/app/src/tools/plane.rs` 截至 2026-07-11 直接使用 contracts `ToolPath`，
  `invoke` 也消费 core `ToolInvocationAction`，内部存储还是
  `BTreeMap<ToolPath, RegisteredTool<C>>`。目标是 app plane 自己定义 path/action，
  并使用 private slot registry + path index；contracts/core 不知道 plane registry key
  或 slot。
- `crates/kernel/src/kernel.rs` 的 `grant_tool_invocation` 截至 2026-07-11 接收 core
  `ToolInvocationAction` 并在 deny 时记录 audit。目标是拆成 generic action grant +
  app orchestration 负责 tool invocation audit。
- `crates/tools/src/file.rs` 的 `ReadFileTool::spec()` 仍返回带 path 的 `ToolSpec`。
  这是 tool descriptor 与 registration path 未拆开的直接症状。
- `crates/tools/src/file.rs` 的 `ReadFileTool::Output` 仍是 `loong_contracts::ToolOutcome`，且
  `build_outcome` 还在 concrete tool 内构造 `status/payload` envelope。目标是
  `ReadFileTool::Output = ReadFileOutput`，只表达成功 payload；失败用 error path 表达。
- `crates/app/src/tools/mod.rs` 里 typed dispatch、grant、invoke、audit 逻辑还堆在
  `execute_kernel_tool_request`。目标是 app orchestration 拥有这段边界，但函数应更聚焦。
- `crates/app/src/tools/routing.rs` 的 `route_direct_read_tool_request_for_legacy` 名字不准。
  它现在同时承担 read surface normalization 和 legacy bridge；aggregate `ReadTool`
  落地后该模块职责应该删除或拆清楚。
- `AppExecutionContext::capabilities()` 目前基本返回 token allowed caps；还没有 child
  tool context 的 effective caps narrowing。
