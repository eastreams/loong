# plan: Kernel / Audit / 当前实现偏差

本文件记录 kernel/audit 的稳定职责，以及截至 2026-07-14 仍存在的实现偏差。

## Kernel Boundary

Kernel 是 governance authority：

- 为新路径运行 capability gate 与 typed policy pipeline；
- 发放 `ActionGrant<A>` / `Granted<A>`；
- 持有 audit sink、clock、authorization/event/grant identity；
- 为仍有 caller 的 legacy fallback 验证 pack/token/revocation/time boundary；
- 不持有 typed ToolPlane，不 dispatch concrete tool，不拥有 app Session/Context。

tool invocation、filesystem operation 和其它 domain intent 都使用现有 `PolicyEngine::grant`。
不要增加 `Kernel::grant`、`AuthorizedToolInvocation`、tool-specific receipt 或只把现有 grant
包一层的 helper。

现有 `loong_core::kernel::Kernel<C>` trait 为 Access 暴露 `policy_engine()`，Access 再调用
`PolicyEngine::grant`；这条 typed path 保持不变。当前错误在 app typed tool invocation：它绕回
接收 pack/token 的 concrete `Kernel::grant_action`。目标是让该 caller 直接复用现有 typed grant，
而不是重写 core Kernel/PolicyEngine ownership。typed authorization error 继续保留
`PolicyGrantError` source；不能降级成 concrete `KernelError` 或字符串。

`CapabilityToken`、`KernelInvocationContext` 和 manual authorization 继续服务仍有 caller 的
legacy fallback，但必须位于明确的 legacy module/bounded impl。typed Kernel contract、Access、
ToolPlane 和 recursive Context 不依赖它们。

## Audit Invariant

audit 分为两个强制边界，owner 与调用路径已经确定：

1. **Authorization evidence**：`PolicyEngine::grant` 是唯一 typed authorization API；它调用
   implementor 的 mandatory audit behavior，不依赖 caller 手写 hook。concrete kernel
   `PolicyPipeline` 持有 kernel-private shared audit state，和 Kernel 共用 sink、clock、attempt/event
   id 与 grant id source；不增加 public wrapper/forwarder。
2. grant 在 capability gate/policy evaluation 前开始 attempt；capability failure、policy deny、
   permission failure 和 allow terminal 都由同一 implementor 发起恰好一次 terminal write。
   allow 先分配 grant id、写 terminal allow event，并在 sink 确认成功后构造、返回
   `ActionGrant`。任何 authorization audit write failure 都必须在 grant 逃逸前返回 typed
   `PolicyGrantError`，保留 sink source 和已经产生的 `PolicyReport`。permission interaction 是
   零到多条关联同一 attempt 的 event。
3. **Execution evidence**：grant consumption owner 记录 completed/failed/input-error/cancelled。tool
   由 runtime `ToolInvocation` wrapper 强制记录并携带 authorization grant id；fs 由 concrete
   `Granted<Action>::run(ctx)` 记录。`ToolImpl` 不获得 audit API。

runtime wrapper 在 dispatch 前完成必要的 execution-start audit write；write 失败时不得 dispatch。
dispatch 后的 terminal write 失败必须显式返回 typed `ToolInvocationError`，但不能抹去 execution
已经 completed、failed 或产生 side effect 的事实，也不得自动重试。dispatch 与 terminal audit
同时失败时，同一个 error variant 必须分别保留两个 typed source。只有 sink 成功接受 terminal
write，才能声称存在对应 terminal execution evidence；audit failure 不能伪造成 terminal
evidence。

Context、caller、concrete policy 和 Access backend 都不写 authorization evidence。legacy pack/token
validation 保留自己的 legacy evidence，不混入 typed attempt schema。

两层不能重复表达同一事实：

- typed capability/policy deny 只属于 typed authorization evidence；legacy token/pack deny 只属于
  legacy evidence；
- tool 内部 fs policy deny 是 fs action authorization deny，同时让外层 tool execution 以 domain
  error 失败；不能伪装成 ToolPlane route/deny；
- concrete tool 不获得裸 audit API；runtime invocation wrapper 自动记录 execution outcome；
- audit 使用 stable display/resource，不把 ToolPlane slot、registry key concrete type、legacy route
  或 fallback 语义固化进 contracts/kernel。

Turn cancellation 另有 execution lifecycle evidence：client disconnect、explicit cancel 和 runtime
shutdown 必须能区分；partial output 不能记录成 completed。已经提交的 side effect 保留各自 action
execution evidence。

## Tool Failure Matrix

- typed capability reject：generic action authorization deny；plane 不执行。
- legacy pack/token reject：legacy authorization deny；不能伪装成 typed action evidence。
- invocation policy deny：generic action authorization deny，包含 `PolicyReport`；plane 不执行。
- payload parse/input error：invocation grant 已消费，记录 tool execution input error；不 fallback。
- concrete tool error：记录 tool execution failed。
- dispatch 前必要 execution-start audit write 失败：返回 typed audit error，不 dispatch。
- dispatch success + terminal audit failure：返回 typed `ToolInvocationError`，保留 execution
  completed 事实与 audit source；execution 可能已产生 side effect，不自动重试。
- dispatch failure + terminal audit failure：同一个 typed error variant 同时保留 dispatch source
  与 audit source；不以其中一个覆盖另一个。
- tool 内部 access deny：记录 domain action authorization deny；外层 tool execution failed。
- cooperative cancellation：停止新 action，记录 tool/turn cancelled；不改写成 policy deny。
- forced abort after grace timeout：记录 cancellation timeout/forced termination，不能假装正常 cancelled。
- legacy fallback：只记录 legacy evidence，直到对应 tool 迁移；typed tests 不接受宽松双断言。

## 当前偏差

### Generic grant evidence 不完整

- `ActionGrantInfo` 已保存完整 allow `PolicyReport`，但 `PolicyEngine::grant` 尚未自动记录完整
  attempt-correlated authorization audit；这是 audit contract 的剩余工作，不构成改写 grant owner
  的理由。
- app typed tool invocation 仍调用接收 pack/token 的 `Kernel::grant_action`。该方法把 typed policy
  deny 转成 legacy `PolicyError` 并记录旧 authorization denial；没有统一记录 action metadata、
  attempt id、report 和 grant id。
- `KernelInvocationContext::request_parameters()` 仍让 legacy `PolicyAny` 从 Context 读取另一份请求
  JSON，而不是读取 `ActionMeta::payload()`。
- permission decision 当前与 allow/deny 一样立即终止 pipeline；registry 尚未编码“hard constraints
  先于 terminal consent”，较早的 permission policy 仍可能跳过后续 typed hard deny。

### Tool execution audit 尚未迁入 runtime wrapper

- `contracts::AuditEventKind::ToolInvocation` 仍是 tool-specific event；
  `Kernel::record_tool_invocation` 仍由 kernel 构造它。
- 当前 event 没有 grant id；typed authorization 与 execution evidence 不能可靠关联。
- owner 已固定为 runtime `ToolInvocation` wrapper；剩余工作是让它使用 kernel-owned audit sink
  capability、带上 grant id，并把 raw granted dispatch 收为 runtime-internal。不能把记录责任留给
  Context/caller，也不能只移动函数名。

### Legacy tool/kernel 路径仍大

- `Kernel` 仍持有 `LegacyToolPlane`、memory/connector/runtime legacy planes 和旧 adapter registration。
- `execute_tool_core` 仍有大量 production/test caller；`ToolCoreRequest` / `ToolCoreOutcome` 仍是
  conversation/session/tool ingress 的主 envelope。
- app static catalog、legacy display alias 和 direct dispatch match 仍与 typed plane metadata 重复。
- `authorize_kernel_action`、`policy_engine_error`、`authorize_operation` 和 control-plane legacy allow
  bootstrap 仍依赖旧 `PolicyError` surface。

### Context 仍是旧 owner

- typed tool invocation 已通过 `AppContext::tool(...).invoke(...)` 构造 concrete action，但 grant
  仍接收 legacy pack/token；`AppContext` 也仍是 Arc/COW session+invocation 混合体。
- `plane` / `tier` 没有真实 Context consumer；`request_parameters` 只服务 legacy policy test/path。
- `ConversationRuntimeBinding` / `ProviderRuntimeBinding` 仍用 optional/advisory 分支传播“可能没有
  Context”。

### Streaming disconnect 不取消执行

- `/v1/chat/completions` streaming 在 detached `tokio::spawn` 中执行完整 turn。
- SSE receiver drop 只使 `sender.send` 失败；provider stream、tool、持久化和 final response 继续运行。
- provider streaming loop 与 retry sleep 没有 Turn cancellation signal；Session/Turn outcome 也没有
  cancelled finalization contract。

### Runtime crate root 仍有旧 spine

- `loong-runtime::runtime::Runtime<C>` 和 `tool_plane` 已经是有效 owner；
- crate root 仍宣称自己是 transitional spine，并保存与 app conversation runtime 重叠的 one-shot /
  interactive contract。这部分需要删除或迁入真实 owner，不能继续与新 Runtime 并存。
