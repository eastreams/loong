# plan: ToolPlane 与 Tool Invocation

本文件记录已经稳定的 typed ToolPlane contract 和尚未删除的 legacy ingress。

## Plane Ownership

- `loong-runtime::tool_plane` 拥有 default plane 的 `ToolPath`、`ToolInvocationAction`、
  `ToolPlane` trait 和 `ToolPlaneRegistry`。
- path 类型是 concrete plane associated type。default `ToolPath` 使用 segment path；另一个 plane
  可以选择 trie key、interned key 或其它表示，contracts/core 不作全局规定。
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

## Invocation Contract

```text
ctx.tool(path)?
  -> resolved ToolInvocation handle
  -> invoke(payload)
  -> validate caps override and derive child Context
  -> ToolInvocationAction(path, required caps, payload)
  -> Kernel generic grant
  -> Granted<ToolInvocationAction>
  -> ToolPlane::invoke(grant, &child_ctx)
  -> RegisteredTool parse typed input
  -> concrete ToolImpl::execute
  -> Value or typed error
```

- `ctx.tool(path)` 只 lookup，不 grant、不 parse payload。
- `ToolInvocation::invoke(payload)` 是普通 caller 唯一入口。它绑定 capability narrowing、generic
  action grant、granted dispatch 和 execution audit。
- `ToolPlane::invoke` 是低层 granted primitive，只接收 grant + Context，不知道 kernel/audit 参数。
- `ToolInvocationAction` 只授权进入 concrete tool。tool 内部 side effect 仍通过 Access 构造新的
  domain action。
- policy 必须看到原始 agent payload，因此 payload 在 grant 前进入 `ToolInvocationAction`，在
  grant 被消费后取回并 parse。
- parse/input error 是 typed invocation failure，不 fallback。
- tool 调 tool 仍经过 `ctx.tool(...).invoke(...)`，child caps 只缩窄，cancellation/mode/goal 等
  Turn identity 继承父 Context。

## Tool Contract

- `ToolImpl<C>` 提供 typed `Input`、typed `Output: Into<Value>`、descriptor、parse 和 execute。
- `ErasedTool` 保持 private/sealed，确保注册 metadata、grant wrapper 和 automatic audit 无法被
  concrete implementer 绕过。
- tool 不拥有 path。provider/catalog 需要 path + descriptor 时由 plane 投影。
- 不存在 `ToolPayloadMatch` / `match_payload`。aggregate `ReadTool` 自己 parse file/query/glob，并
  调用不同 fs operation/action。
- app-owned output observer 可以在 typed output erase 前处理 preview 等 app side channel；observer
  不属于 concrete tool crate，也不能做未经 Access 治理的副作用。

## Legacy 删除目标

当前仍存在大量 `ToolCoreRequest` / `ToolCoreOutcome`、`Kernel::execute_tool_core`、legacy adapter
和 static catalog 调用面。剩余迁移必须满足：

- 持有 unified Context 的 caller 直接调用 `ctx.tool(path)?.invoke(payload).await`，不先包装 legacy
  envelope。
- 尚未迁移的 legacy tool 只从旧 ingress 最末端 fallback；不能注册进 typed plane冒充迁移。
- concrete descriptor 迁入 tool/registration owner 后，删除 app static catalog 重复 metadata。
- display name 来自 plane-local path formatter；删除 `file.read -> read` 等 display alias helper。
- 所有 concrete tools 迁完后删除 `ToolCoreRequest` / `ToolCoreOutcome`、`LegacyToolPlane`、
  `CoreToolAdapter` / `ToolExtensionAdapter` 和 `Kernel::execute_tool_core`。

完成后的新增 builtin tool 只改两处：

1. concrete type + `impl ToolImpl<C>`；
2. app bootstrap 中一条 `register(path, tool)`。
