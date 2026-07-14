# plan: Kernel / Audit / 当前实现偏差

本文件记录 kernel/audit 的稳定职责，以及截至 2026-07-15 仍存在的实现偏差。

## Kernel Boundary

Kernel 是 governance authority：

- 为新路径运行 capability gate 与 typed policy pipeline；
- 发放 `ActionGrant<A>` / `Granted<A>`；
- 持有 audit sink、clock、authorization/event/grant identity；
- 为仍有 caller 的 legacy fallback 验证 pack/token/revocation/time boundary；
- 不持有 typed ToolPlane，不 dispatch concrete tool，不拥有 app Session/Context。

tool invocation、filesystem operation 和其它 domain intent 都使用现有 `PolicyEngine::grant`。
不要增加 `Kernel::grant`、`AuthorizedToolInvocation`、tool-specific receipt 或只把现有 grant
包一层的 helper。也不要增加 `AuditHandle`、route/receipt 或 `ctx.audit`。

现有 `loong_core::kernel::Kernel<C>` trait 为 Access 暴露 `policy_engine()`，Access 再调用
`PolicyEngine::grant`；返回值是 opaque `&impl PolicyEngine<C>`，跨 crate caller 看不到 installed
pipeline 的 backend hooks。这条 typed Access path 已自动获得 mandatory authorization evidence。
当前错误在 app typed tool invocation：它仍绕回接收 pack/token 的 concrete
`Kernel::grant_action`。步骤 5 让该 caller 直接复用 typed grant，并删除没有 legacy production
caller 的 `Kernel::grant_action`。typed authorization error 继续保留 `PolicyGrantError` source；不能
降级成 concrete `KernelError` 或字符串。

`CapabilityToken`、`KernelInvocationContext` 和 manual authorization 继续服务仍有 caller 的
legacy fallback，但必须位于明确的 legacy module/bounded impl。typed Kernel contract、Access、
ToolPlane 和 recursive Context 不依赖它们。

## Audit Invariant

audit 分为 authorization 与 execution 两类 evidence。authorization owner 和当前 active goal 的
ToolInvocation owner 已确定；generic Access execution owner 尚未确定：

1. **Authorization evidence**：`PolicyEngine::grant` 是唯一 typed authorization API；capability
   gate、policy evaluation、mandatory audit 和 mint 顺序由 core 固定，对外不可覆写。
   `PolicyEngine` 对 caller 只暴露 `grant`；decision、audit write 与 identity source 收在窄 backend
   contract 中，core 可以在其上 blanket 实现该 trait，但不依赖 kernel `AuditError`。
   `PolicyContext` 只读提供 owned typed authorization subject/identity，不提供 sink 或 `ctx.audit`。
   public `loong_kernel::policy::PolicyPipelineBuilder` 只注册 policy，kernel crate root 不 re-export
   它；Kernel 将 builder 和 non-optional shared audit state 安装成 private `PolicyPipeline`，与
   Kernel 共用 sink、clock、attempt/event id 与 grant id source。builder 不实现 `PolicyEngine`，
   不存在 unbound runnable pipeline，也不增加 public wrapper/forwarder。Kernel 不提供 silent/no-op
   audit constructor；测试使用可观察的 in-memory sink。
   subject scope 只有真实 `Session { session_id }` 与显式隔离的
   `LegacyToken { boundary, pack_id, token_id }`；后者让 legacy token/pack audit filter 有类型化
   correlation，同时不把 bearer 字段扩散进普通 typed Context。
2. `AuthorizationEvidence` 用嵌套 sum type 编码阶段，而不是允许任意组合的 optional 字段：
   `StartFailed` 不能携带 attempt id 或后续 event；`Started { id, CapabilityDenied }` 不携带 policy
   report；policy 已运行时只使用 `Started { id, Policy { report, event } }`，其中 event 是 permission
   interaction 或 terminal outcome。这从类型上排除 `StartFailed + Allow`、permission without report
   等不可能状态。permission interaction 进一步用 `Approved`、`Denied` 和 `EscalatedToUser` 分支
   排除 `User + Escalate`。
3. grant 在 capability gate 前分配 attempt id。allocation 失败时只能尝试写 `StartFailed`；成功后
   capability deny、policy deny、permission failure 和 allow 都复用同一 attempt id。allow 先分配
   grant id、写 terminal allow event，并在 sink 确认成功后把同一 id 写入 outer `ActionGrant.id`，
   再构造、返回 grant。permission interaction 是零到多条关联同一 attempt 的 event。
   `PolicyGrantError::Audit` 保留普通 evidence write failure；`IdentityAllocation` 保留 allocation
   failure；记录 allocation failure 也失败时，`IdentityAllocationAndAudit` 同时保留 allocation 与
   audit source。每个 error 携带 core 准备写入的 exact evidence，但不能声称 sink 已经接受它。
   任一失败都必须发生在 grant 逃逸前。当前 outer `ActionGrant` 已有 id/info，当前 goal 不修改
   `Granted` 字段形状或增加 `grant_id()`。
4. **Execution evidence**：grant consumption owner 记录 completed/failed/input-error/cancelled。tool
   在当前 active goal 由 runtime `ToolInvocation` wrapper 保留 outer `ActionGrant.id/info`，直到关联
   execution audit 结束，并用 outer id 关联 execution outcome。generic `Granted<Action>` / Access
   execution evidence 尚未实现，留给独立后续目标；`ToolImpl` 不获得 audit API。

`FanoutAuditSink` 只保证 engine 对配置的 sink 发起一次 write 调用。它不提供跨 child sink 的
transaction、rollback 或 retry；某个 child 已接受而后续 child 失败时，error 必须保留这一事实，
engine 不得虚构全局原子提交或自动重试。

attempt、grant 与 generic audit event identity exhaustion 使用独立 typed `AuditError` variant；不能
退回解析 `AuditError::Sink(String)` 来判断失败种类。

runtime 不得访问 kernel-private audit state。跨 crate execution evidence 只通过 Kernel 现有 generic
`record_audit_event` governance recorder 写入；保留该方法并将其 error boundary 收敛为 typed
`AuditError`。它负责 clock、event id 与 sink write，有真实 ownership 职责，不是 forwarding helper。
该外部 recorder 明确拒绝 `AuditEventKind::Authorization`；typed authorization 只能由 sealed grant
algorithm 经 private backend 写入。当前拒绝使用专门的
`AuditError::AuthorizationEvidenceOwnedByPolicyEngine`，不能退回
`AuditError::Sink(String)` 或字符串分类。

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
shutdown 必须能区分；partial output 不能记录成 completed。cancellation 不回滚已经提交的 side
effect。当前 active goal 只闭合 ToolInvocation outcome；generic Access action 的
completed/failed/cancelled evidence 与“side effect already happened”语义留给独立后续目标，在此
之前不能声称 Access execution evidence 已存在。

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

sealed grant algorithm 与 authorization evidence contract 已闭合；active goal 在 audit 维度只剩
runtime ToolInvocation execution evidence。不在该目标中偷做 generic Access execution audit。

## 当前偏差

### Tool execution audit 尚未迁入 runtime wrapper

- `contracts::AuditEventKind::ToolInvocation` 仍是 tool-specific event；
  `Kernel::record_tool_invocation` 仍由 kernel 构造它。
- 当前 event 没有 grant id；typed authorization 与 execution evidence 不能可靠关联。
- owner 已固定为 runtime `ToolInvocation` wrapper；剩余工作是让它保留 outer grant metadata、通过
  generic `record_audit_event` 获得 typed `AuditError`，并把 raw granted dispatch 收为
  runtime-internal。不能把 kernel-private state 暴露给 runtime，也不能只移动函数名。

### Hard constraint 与 terminal consent 尚未分段

- permission decision 当前与 allow/deny 一样立即终止 pipeline；registry 尚未编码“所有 hard deny
  先于 terminal consent”。较早的 permission policy 仍可能跳过后续 typed hard deny。
- production 在该阶段边界完成前不得注册会返回 permission decision 的 policy；这不影响已经闭合的
  capability/policy/permission authorization evidence 顺序。

### Generic Access execution evidence 尚未实现

- 当前 `Granted<Action>::run(ctx)` 只调用 `Action::run`；它不自动写 execution audit。该调用签名
  只接收 `Granted`，但它是否就是未来唯一 execution owner 尚未确认。
- 因此 filesystem 和其它 Access action 当前只有各自 policy/grant 结果，没有 generic
  completed/failed/cancelled execution evidence。当前 active goal 不得把 authorization evidence
  误写成 execution evidence。
- 当前 outer `ActionGrant` 已有 id/info，当前 goal 不修改 `Granted` 字段形状。generic execution
  owner/调用签名仍未确定；不把 sink 挂到 Context 或 `Granted`，不增加 `Kernel::grant`，也不借
  ToolInvocation wrapper 偷渡 generic Action owner。

### Legacy tool/kernel 路径仍大

- `Kernel` 仍持有 `LegacyToolPlane`、memory/connector/runtime legacy planes 和旧 adapter registration。
- `execute_tool_core` 仍有大量 production/test caller；`ToolCoreRequest` / `ToolCoreOutcome` 仍是
  conversation/session/tool ingress 的主 envelope。
- app static catalog、legacy display alias 和 direct dispatch match 仍与 typed plane metadata 重复。
- `policy_engine_error`、`authorize_operation` 和 control-plane legacy allow bootstrap 仍依赖旧
  `PolicyError` surface。

### Context 仍是旧 owner

- typed tool invocation 已通过 `AppContext::tool(...).invoke(...)` 构造 concrete action，但 grant
  仍接收 legacy pack/token；`AppContext` 也仍是 Arc/COW session+invocation 混合体。
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
