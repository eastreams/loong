# plan: ToolPlane 与 Tool Invocation

本文件记录已经稳定的 typed ToolPlane contract 和尚未删除的 legacy ingress。

## Plane Ownership

- `loong-runtime::tool_plane` 拥有 default plane 的 `ToolPath`、`ToolInvocationAction`、
  runtime-internal `ToolPlane` trait、`ToolPlaneRegistry` 以及
  `error::{RegistrationError, LookupError<P>, DispatchError<P>}`。
- runtime `ToolInvocation` wrapper 与 `ToolInvocationContext` 也必须归属 `loong-runtime`；当前
  app-owned wrapper 是步骤 5 要删除的实现偏差，不能再增加第二个 bridge。
- `ToolPlane` 不是外部扩展点。它只允许 runtime crate 内部替换 registry/storage strategy；default
  `ToolPath` 使用 segment path，contracts/core 不作全局规定。`Runtime` 不接受 caller-provided plane，
  也不向 crate 外暴露能消费 invocation grant 的 trait object。
- app bootstrap 注册 concrete builtin tools 和 app-owned policy/success observer；kernel 不持有
  typed registry，concrete tool crate 不持有 registry。
- `Runtime<C>` 持有构造完成的 plane。registration 是 fallible bootstrap，duplicate path 不能变成
  lazy global panic。

## Registry Invariant

default registry 已采用 private slot storage + ordered path index：

```text
ToolPath -> private ToolSlot -> RegisteredTool<C>
```

- `ToolSlot` 不跨 module boundary，不进入 Action、audit、contracts/core 或 concrete tool API。
- entry 不重复保存 path，避免 index 与 entry drift。
- `RegisteredTool` 保存 descriptor、registration time 和 provenance；private `ErasedTool` 只由
  `RegisteredTool` 构造。
- path alias、unregister、Trie 或 plugin replacement 不是当前 contract。出现真实需求时单独设计，
  不用 legacy alias helper 偷渡。

## 当前 Typed Error Boundary

当前 typed tool primitive 已经固定以下错误边界：

- `ToolImpl<C>` 有 associated `Error: Error + Send + Sync + 'static`，concrete tool error 不再先
  压成字符串；
- private erasure 使用 `RegisteredToolError::{Input, Execution}`，其中 execution variant 保留
  concrete tool error source；
- runtime plane 分开拥有 `RegistrationError`、`LookupError<P>` 与 `DispatchError<P>`，lookup 和
  granted dispatch 的 registry invariant 不再混成同一个 fallback signal；
- 只有 `LookupError::NotRegistered` 可以由 legacy ingress 选择 fallback；
  `LookupError::RegistryInvariant`、任何 `DispatchError` 和 `RegisteredToolError` 都不得 fallback。

composite `ToolInvocationError` 尚未定义。它只能在步骤 5 把 runtime wrapper、child narrowing、
direct grant、granted dispatch 和 execution audit 真正接线的同一个 owner-driven 提交中定义并被
调用路径使用；不安排独立 public error scaffold commit。

## Invocation Contract

步骤 5 完成后的 contract 是：

```text
ctx.tool(path)?
  -> thin Context::tool entry asks Runtime for ToolInvocation handle
  -> invoke(payload)
  -> validate caps override
  -> ToolInvocationContext::derive_tool_child(capabilities)
  -> ToolInvocationAction(path, required caps, payload)
  -> PolicyEngine::grant (mandatory authorization audit)
  -> ActionGrant<ToolInvocationAction>
  -> wrapper retains outer ActionGrant.id/info through execution audit
  -> runtime-internal dispatch consumes Granted<ToolInvocationAction>
  -> RegisteredTool parse typed input
  -> concrete ToolImpl::execute
  -> runtime wrapper submits execution evidence through Kernel::record_audit_event
  -> Value or ToolInvocationError
```

- `Context::tool(path)` 是 app concrete Context 上的薄入口，只调用 Runtime 创建 handle；不在 app
  复制 lookup、narrowing、grant、dispatch 或 audit orchestration。
- `ToolInvocation::invoke(payload)` 是普通 caller 唯一入口。它绑定 capability narrowing、generic
  action grant、granted dispatch 和 execution audit。
- raw granted dispatch 是 `loong-runtime` 内部 primitive，只接收 `Granted` + Context，不知道
  kernel/audit 参数，也不能作为 public/普通 caller API。步骤 5 破坏性删除 public `ToolPlane::invoke`、
  generic `Runtime::new<P>` 和 `Runtime::tools() -> dyn ToolPlane`；catalog/spec 查询通过不暴露 dispatch
  capability 的 Runtime API 提供。
- runtime wrapper 在消费 `granted` 前保留 outer `ActionGrant.id/info`，直到关联 execution audit
  结束；当前目标不把 metadata 复制进 `Granted`。
- composite wrapper 接线后，lookup、caps override validation、child narrowing、
  `PolicyEngine::grant`、dispatch 和 execution audit 全部返回 `ToolInvocationError` 并保留 typed
  source；`PolicyGrantError` / `AuditError` 不转换成 `KernelError` 或字符串。
- runtime wrapper 不读取 kernel-private sink/clock/id state；它只向 Kernel 现有 generic
  `record_audit_event` 提交 evidence kind/attribution。保留该 recorder，并将其 error boundary 收敛为
  typed `AuditError`；recorder 自己负责 clock、event id 与 sink write。
- `ToolInvocationAction` 只授权进入 concrete tool。tool 内部 side effect 仍通过 Access 构造新的
  domain action。
- policy 必须看到原始 agent payload，因此 payload 在 grant 前进入 `ToolInvocationAction`，在
  grant 被消费后取回并 parse。
- parse/input error 是 typed invocation failure，不 fallback。
- tool 调 tool 仍经过 `ctx.tool(...).invoke(...)`，child caps 只缩窄，cancellation/mode/goal 等
  Turn identity 继承父 Context。nested tool 只能编排，最终物理 side effect 仍必须进入
  `Granted` Access action。

## Context Requirement

`ToolInvocationContext` 是 `loong-runtime` 拥有的窄 requirement trait，也是 runtime
`ToolInvocation` 与 app Context 的唯一直接 contract：

- app concrete Context 实现该 trait；
- 唯一 operation `derive_tool_child` 按给定 `Capabilities` 从 parent 派生同类型 child，返回
  `Result<Self, CapabilityNarrowingError>`；trait 没有 Factory 参数或 associated error；
- trait 不暴露 kernel、audit 或 Runtime，不属于 `ContextFactory`，也不构造 base Context；
- runtime `ToolInvocation` 使用它完成 narrowing，然后在 authorization audit 已闭合的前提下调用
  `PolicyEngine::grant`、runtime-internal plane dispatch 和 execution audit。

trait 附近必须注释 why：它跨 crate 表达 child authority narrowing，不是只搬运参数或缩短调用的
helper。不能再增加第二个 app/runtime Context bridge。

## Tool Contract

- `ToolImpl<C>` 提供 typed `Input`、typed `Output: Into<Value>`、associated `Error`、descriptor、
  parse 和 execute。
- `ErasedTool` 保持 private/sealed，确保 concrete implementer 不能注入绕过 input parsing 或丢失
  error source 的 erasure；grant wrapper 与 automatic audit 由步骤 5 的 runtime invocation owner
  强制，不能错误归因给 `ErasedTool` 本身。
- tool 不拥有 path。provider/catalog 需要 path + descriptor 时由 plane 投影。
- 不存在 `ToolPayloadMatch` / `match_payload`。aggregate `ReadTool` 自己 parse file/query/glob，并
  调用不同 fs operation/action。
- app-owned output observer 可以在 typed output erase 前处理 preview 等 app side channel；observer
  不属于 concrete tool crate，也不能做未经 Access 治理的副作用。observer 保留 `()` 返回值，
  没有 recoverable `Err` channel；callback 不得 panic，registrar 必须在 callback 内吸收或处理可
  恢复的 delivery failure，不能在 tool side effect 已完成后改写 typed success。

## Legacy 删除目标

当前仍存在大量 `ToolCoreRequest` / `ToolCoreOutcome`、`Kernel::execute_tool_core`、legacy adapter
和 static catalog 调用面。剩余迁移必须满足：

- 持有 unified Context 的 caller 直接调用 `ctx.tool(path)?.invoke(payload).await`，不先包装 legacy
  envelope。
- 尚未迁移的 legacy tool 只在 typed lookup 明确返回“path 未注册”时从旧 ingress 最末端
  fallback；caps override、narrowing、grant、parse/input 和 execution error 一律不 fallback。
  legacy tool 不能注册进 typed plane 冒充迁移。
- concrete descriptor 迁入 tool/registration owner 后，删除 app static catalog 重复 metadata。
- display name 来自 plane-local path formatter；删除 `file.read -> read` 等 display alias helper。
- 所有 concrete tools 迁完后删除 `ToolCoreRequest` / `ToolCoreOutcome`、`LegacyToolPlane`、
  `CoreToolAdapter` / `ToolExtensionAdapter` 和 `Kernel::execute_tool_core`。

完成后的新增 builtin tool 只改两处：

1. concrete type + `impl ToolImpl<C>`；
2. app bootstrap 中一条 `register(path, tool)`。
