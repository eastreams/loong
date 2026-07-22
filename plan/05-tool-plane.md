# plan: ToolPlane 与 Tool Invocation

本文件记录已经确定的 typed ToolPlane 目标 contract 和尚未删除的 legacy ingress；当前未满足项只在
`07-kernel-audit-and-deviations.md` 与 `08-next-steps.md` 列出。

## Plane Ownership

- `loong-contracts` 拥有唯一 `ToolPath` identity；`loong-runtime::tool_plane` 拥有
  `ToolInvocationAction`、`ToolPlaneRegistry` 以及
  `error::{RegistrationError, LookupError, CapabilityOverrideError, ToolInvocationError}`。
- runtime `ToolInvocation` 与 concrete `Context<'a>` 都归属 `loong-runtime`。generic ToolInvocation
  通过窄 `ToolInvocationContext` requirement 派生 child；该 trait 不构造 root Context，也不增加
  app-owned invocation wrapper。
- `ToolPlaneRegistry` 不是外部 dispatch 扩展点。`Runtime` 直接持有构造完成的 concrete registry；
  `BTreeMap` 或未来其它 storage strategy 只在 registry 内部替换，不为此预造 forwarding trait。
  path identity、比较和序列化不随 BTreeMap/Trie 实现变化。Runtime 不接受可替换 plane trait
  object，也不向 crate 外暴露能消费 invocation grant 的对象。
- `RegisteredTool`、registered descriptor owner、`ErasedTool` 和 raw erased invoke 都属于
  `loong-runtime`。`loong-core` 只暴露 `ToolImpl`；app bootstrap 通过 registry registration API 交入
  concrete tool，但任何 crate 外 caller 都不能构造 registered entry 或直接 dispatch。
- app bootstrap 注册 concrete builtin tools 和 app-owned policy/success observer；kernel 不持有
  typed registry，concrete tool crate 不持有 registry。
- concrete `Runtime` 持有 `ToolPlaneRegistry<RuntimeContextFactory>`。registration 是 fallible
  bootstrap，duplicate path 不能变成 lazy global panic。

`ToolPath` 是有序、非空的 opaque segments；每个 segment 非空且不含 `/` 或控制字符。canonical
text/serde 固定为 `/a/b/c/tool`，大小写敏感且不做 Unicode normalization。`.`、`..` 只是逻辑
segment，没有 filesystem navigation 语义。当前 dotted tool name 是一个完整 segment，例如
`glob.search -> /glob.search`，不能隐式拆成 `/glob/search`；真正的层级 namespace 必须在注册时显式
传入多个 segments。contracts 不提供 `From<&str>`、`From<String>` 或 dotted split conversion。

## Registry Storage

default registry 直接使用 ordered path map。唯一 lookup chain 是：

```text
ToolPath -> &RegisteredTool<C>
                |
                +-> borrowed ToolInvocation handle
```

- 当前没有 unregister、replacement 或 stable slot caller，因此第二套 storage identity 没有语义价值。
  真正出现该需求前不引入 slot arena、generation 或 path-to-slot index。
- entry 不重复保存 path，避免 index 与 entry drift。
- registration 使用 `Direct { provider_name } | Discoverable { discovery_name }` 和类型分支表达
  exposure；这些字符串只负责 presentation/invocation projection，不是 plane identity。request-scoped
  provider surface 只为 Direct 分支建立 wire name -> exact path 投影。
- runtime-private `RegisteredTool` 直接保存 descriptor 与 private `ErasedTool`。不为尚无消费者的
  registration time/provenance 预造 wrapper；未来若 audit 或 replacement 确实需要 registration metadata，
  只能在该 registration owner 处加入。crate 外 metadata query 返回不带 raw invoke 的 view，不能返回
  entry 本身。
- `Context::tool` 使用 runtime-private tool service 在 grant 前完成这次 lookup，并把
  `&RegisteredTool<RuntimeContextFactory>` 绑定进
  `ToolInvocation`。handle 另外保存 contracts-owned path 供 Action/audit 使用；path 不能在 grant 后重新
  充当 storage lookup key。
- runtime 持有构造完成后不再 mutation 的 registry，因此一个活着的 entry borrow 已经证明 storage
  entry 存在。registry 不提供 async invoke；runtime wrapper 消费 grant 后直接调用 bound
  `RegisteredTool`。
- 单表 lookup 只有 `LookupError::NotRegistered`；不存在由双表漂移制造的 registry invariant error。
- path alias、unregister、Trie 或 plugin replacement 不是当前 contract。出现真实需求时单独设计，
  不用 legacy alias helper 偷渡。
- Session 的 `ToolView` 以 concrete runtime `ToolPath` 为 authority index，只额外保存 provider/catalog
  字符串投影。typed visibility policy 直接查询 path；不能把 path 降成字符串后在授权时重新 split。

## Typed Error Boundary

typed tool primitive 必须固定以下错误边界：

- `ToolImpl<C>` 有 associated `Error: Error + Send + Sync + 'static`，concrete tool error 不再先
  压成字符串；
- runtime-private erasure 使用 `RegisteredToolError::{Input, Denied, Execution}`：`Denied` 在唯一 erasure
  boundary 保留 nested typed authorization source，`Execution` 保留其它 concrete tool error source；
- runtime plane 分开拥有 `RegistrationError` 与 `LookupError`；lookup invariant 与 concrete tool
  input/execution error 不混成同一个 fallback signal；
- 只有 `LookupError::NotRegistered` 可以由 legacy ingress 选择 fallback；
  任何 `RegisteredToolError` 都不得 fallback；
- 首次 lookup 绑定 entry 后，`RegisteredToolError` 直接成为 `ToolInvocationError` 的 dispatch source。
  不保留只包一层 tool error 的 `DispatchError`，post-grant persisted schema 也不保存 registry
  invariant。
- concrete tool 内部的 Access authorization denial 必须在 type erasure 后仍可被 orchestration 识别为
  policy denial。拥有 concrete error conversion/erasure 的边界可以沿 `Error::source()` 检查一次 nested
  typed authorization error，并立刻固化成 `RegisteredToolError::Denied`；turn engine 只 match 该 variant，
  不能枚举每个 Access error、再次扫描 source chain 或判断 display string。
- typed runtime 使用 `#[non_exhaustive] ActionExecutionEvent` 表达 Started、Completed、InputRejected、
  Failed 与 OutcomeUnknown，并只携带关联真实 grant 的 `GrantId`。旧 `AuditEventKind::ToolInvocation`
  只用于解码 pre-grant-linked 历史日志；未来 cancellation 等真实状态扩展前者，不预造同构 enum。

typed 调用保留两个 fallible API 阶段：

- `Context::tool(path)` 返回 concrete `LookupError`；
- `ToolInvocation::with_capabilities_override(...)` 只保存 requested value 并返回 handle；
- `ToolInvocation::invoke(payload)` 返回 `ToolInvocationError`，其 variants 覆盖 governed override
  rejection、override rejection + audit failure、child narrowing、authorization、start-audit、dispatch、
  completed + terminal-audit，以及 dispatch + terminal-audit。

`ToolInvocationError` 的每个 variant 保留 typed source 和必要的 `GrantId`/completed fact。临时 app
ingress 用 `ToolRequestError` 组合 input/context、lookup、invocation 与 explicit legacy
failure；这是 typed/legacy 共存期的 owner，不是新的 runtime error。turn boundary 可以据此分类 deny、
retryable input 与 non-retryable execution/infrastructure failure，但不能先压成 `KernelError` 或字符串。
metadata lookup 也只有 `LookupError::NotRegistered` 能选择 legacy/static metadata。

## Invocation Contract

当前 typed invocation contract 是：

```text
ctx.tool(path)?
  -> runtime tool service resolves path once and binds &RegisteredTool into ToolInvocation
  -> optional with_capabilities_override(...)
  -> invoke(payload)
  -> validate requested override and audit rejection without dispatch
  -> required caps = InvokeTool + selected tool caps
  -> derive child with parent caps intersected with required caps
  -> ToolInvocationAction(path, full required caps, payload)
  -> PolicyEngine::grant (mandatory authorization audit)
  -> ActionGrant<ToolInvocationAction>
  -> ActionGrant::into_granted
  -> Granted<ToolInvocationAction>::run(runtime-private execution context)
  -> ToolInvocationAction::run reads the same proof's id/info and records Started
  -> bound RegisteredTool parses typed input
  -> concrete ToolImpl::execute
  -> ToolInvocationAction::run records terminal evidence through the same grant-bound recorder
  -> Value or ToolInvocationError
```

- `Context::tool(path)` 是 runtime concrete Context 上的薄入口，只使用 Session runner 安装的窄 tool
  service 创建 handle；它不取得 `runtime::Handle`，也不让 app 复制 lookup、narrowing、grant、
  dispatch 或 audit orchestration。
- `ToolInvocation::invoke(payload)` 是普通 caller 唯一入口。它绑定 capability narrowing、generic
  action grant、granted dispatch 和 execution audit。
- grant consumption + registered entry dispatch 只存在于 `loong-runtime::ToolInvocation` 内部，不知道
  legacy token/pack，也不能作为 public/普通 caller API。registry 不消费 grant；
  `Runtime::new` 只接收 concrete registry，catalog/spec 查询通过不暴露 entry dispatch capability 的
  Runtime API 提供。
- `Granted<ToolInvocationAction>` 携带同一次 mint 的 UUID id、authorization info 和 action；
  `ToolInvocationAction::run` 在消费该 proof 的生命周期内完成 Started/terminal correlation。不存在
  outer id copy、receipt、active-state map 或第二套 metadata path；persisted execution event 仍只写
  grant id，不重复 authorization report。
- lookup 单独返回 `LookupError`；requested override 只在 `invoke` 内校验并由
  `ToolInvocationError::CapabilityOverride` 保留 source/evidence。child narrowing、
  `PolicyEngine::grant`、bound entry execution 与 execution audit 同样由 `ToolInvocationError` 保留
  source。`PolicyGrantError` / `RegisteredToolError` / `AuditError` 不转换成 `KernelError` 或字符串。
- child Context 只获得 `parent ∩ required`，但 Action 仍声明完整 required caps。缺失 capability 必须
  在 `PolicyEngine::grant` 中拒绝并自动留下 authorization evidence，不能被提前 narrowing 短路。
- concrete Context 的 private child derivation 自身拒绝扩权；runtime 仍验证派生结果不超出 selected
  subset，防止实现错误把 tool 未声明的 parent capability 带进 child。
- runtime wrapper 不读取 kernel-private sink/clock/id state。generic operational recorder 拒绝
  `Authorization`、grant-bound `ActionExecution` 与只读历史 `ToolInvocation`；runtime 通过绑定真实
  invocation `Granted` 的窄 recorder 提交 Started/terminal `ActionExecution` evidence。Kernel 负责
  clock、event id 与 sink write，error 保持 typed `AuditError`。
- `ToolInvocationAction` 只授权进入 concrete tool。tool 内部 side effect 仍通过 Access 构造新的
  domain action。action type 可以 public 供 typed policy 匹配，但 constructor 保持 runtime-private；
  普通 caller 不能自行构造 action 再取得一个可用于 tool execution evidence 的 grant。
- policy 必须看到原始 agent payload，因此 payload 在 grant 前进入 `ToolInvocationAction`，在
  grant 被消费后取回并 parse。
- parse/input error 是 typed invocation failure，不 fallback。
- `tool.invoke` 的 `capabilities_override` 是 outer ingress authority input。任何提前解包 outer envelope
  的 prepare 层都必须把它作为 typed field 保留到 runtime invocation；不能只搬 inner
  `ToolCoreRequest`，否则 typed target 会恢复 default caps，legacy target 也会绕过 fail-closed。
- legacy `tool.invoke` envelope 只解析一次：lease validation、catalog resolution、payload
  normalization 得到已有的 `ResolvedToolExecution + ToolCoreRequest`，typed lookup 与 legacy fallback
  都复用这份结果。typed miss 后必须按首次解析出的 `execution_kind` 进入正确 legacy owner，不能把
  原始 `tool.invoke` envelope 再交给旧 plane 重跑上述步骤。若 target 未注册且请求携带 capability
  override，必须 fail closed；legacy plane 不支持该 narrowing，不能静默丢弃 override 后执行。
- tool 调 tool 仍经过 `ctx.tool(...).invoke(...)`，child caps 只缩窄，cancellation/mode/goal 等
  Invocation-scoped identity 继承父 Context。nested tool 只能编排，最终物理 side effect 仍必须进入
  `Granted` Access action。

## Context Requirement

`ToolInvocationContext` 是 generic `ToolInvocation<C>` 对 `C::Cx<'a>` 的窄 requirement：

- concrete runtime Context 实现该 trait；
- 唯一 operation 按给定 `Capabilities` 从 parent 派生 authority 不扩大的同类型 child，返回 typed
  `CapabilityNarrowingError`；
- trait 没有 factory method，不暴露 kernel、audit、Runtime 或 root construction，也不进入
  `ContextFactory`；
- ToolInvocation 在 child 建立后调用 `PolicyEngine::grant`、bound entry dispatch 和 execution audit；
- Action/Access/Policy 仍使用其 owner 定义的小 requirement trait，不增加一个大 Context deps trait。

## Tool Contract

- `ToolImpl<C>` 提供 typed `Input`、typed `Output: Into<Value>`、associated `Error`、descriptor、
  parse 和 execute。
- `RegisteredTool` 与 `ErasedTool` 都保持 runtime-private；raw invoke 只能由同 crate 的
  `ToolInvocation` 调用。这同时防止 concrete implementer 注入绕过 input parsing 的 erasure，也防止
  普通 caller 绕过 grant/audit。只把 trait 设为 private、却公开 registered constructor/invoke，不算
  sealed boundary。
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
- 已注册 tool 在 validation 与 legacy direct payload router 之前完成 typed ownership 判断；其 payload 只由 concrete
  `ToolImpl::parse_input` 解释，invalid input 经 granted runtime path 记录 `InputRejected`。手工
  `route_direct_tool_name` 只服务 `LookupError::NotRegistered` 后的 legacy request，不能成为 typed
  parser 的前置分支。
- fallback decision 只能由 app ingress 做一次。进入 legacy plane 后不得再次调用
  `ctx.tool(...)`，也不得恢复 context-aware legacy re-entry；否则同一请求会重复
  authorization/audit。
- typed miss 后，app target 交给真实 app legacy dispatcher，core target 交给 runtime 内明确隔离的 legacy
  ingress owner；ordinary Runtime/Context API 不暴露 raw Kernel。
  尚未 dispatch 任何 legacy plane 时不能手工构造 `Legacy(KernelError::ToolPlane)` 冒充 kernel
  execution failure；routing failure 由实际 ingress owner 类型化表达。
- concrete descriptor 迁入 tool/registration owner 后，删除 app static catalog 重复 metadata。
  当前已注册的 `read`、`write`、`edit`、`glob.search`、`content.search` 都必须只从 runtime registration
  投影 provider/search/prompt metadata。不能维护另一个 hard-coded “typed-only names” helper；缺失
  registration 时不得由 static catalog 静默复活 schema。
- identity display 来自 contracts-owned canonical `ToolPath` formatter；provider/discovery presentation
  来自 registration metadata，不从 path 文本反推。删除 `file.read -> read` 等 display alias helper。
- 所有 concrete tools 迁完后删除 `ToolCoreRequest` / `ToolCoreOutcome`、`LegacyToolPlane`、
  `CoreToolAdapter` / `ToolExtensionAdapter` 和 `Kernel::execute_tool_core`。

完成后的新增 builtin tool 只改两处：

1. concrete type + `impl ToolImpl<C>`；
2. app bootstrap 中一条 `register(path, tool)`。
