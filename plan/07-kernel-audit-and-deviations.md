# plan: Kernel / Audit / 当前实现偏差

本文件记录 kernel/audit 的稳定职责，以及必须随代码迁移同步删除或改写的当前实现偏差。

## Kernel Boundary

Kernel 是 governance authority：

- 为新路径运行 capability gate 与 typed policy pipeline；
- 发放 `ActionGrant<A>` / `Granted<A>`；
- 持有 audit sink、clock、authorization/event/grant identity；
- 为仍有 caller 的 legacy fallback 验证 pack/token/revocation/time boundary；
- 不持有 typed ToolPlane，不 dispatch concrete tool，不拥有 Runtime/Session/Context。

tool invocation、filesystem operation 和其它 domain intent 都使用现有 `PolicyEngine::grant`。
不要增加 `Kernel::grant`、`AuthorizedToolInvocation`、tool-specific receipt 或只把现有 grant
包一层的 helper。也不要增加 `AuditHandle`、route/receipt 或 `ctx.audit`。

现有 `loong_core::kernel::Kernel<C>` trait 为 Access 暴露 `policy_engine()`，Access 再调用
`PolicyEngine::grant`；返回值是 opaque `&impl PolicyEngine<C>`，跨 crate caller 看不到 installed
pipeline 的 backend hooks。这条 typed Access path 已自动获得 mandatory authorization evidence。
runtime typed tool 也直接复用该 grant；typed path 不得增加 `Kernel::grant_action` 或同义 forwarding
method。typed authorization error 保留 `PolicyGrantError` source，不降级成 concrete `KernelError` 或
字符串。

`CapabilityToken` 与 manual authorization 继续服务仍有 caller 的 legacy fallback，但必须位于明确的
legacy module/bounded impl。源码已不存在 `KernelInvocationContext`；typed Kernel contract、Access、
ToolPlane 和 recursive Context 不依赖 bearer evidence。

## Audit Invariant

audit 分为 authorization 与 execution 两类 evidence。authorization owner 与 ToolInvocation
execution owner 已确定；generic Access execution owner 尚未确定：

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
   UUID grant id，并从同一次 evaluation 构造 `ActionGrantInfo { report, subject, action }`。terminal allow
   event 写入成功后，core 把 id/info/action 一次性 private-mint 进 `Granted<A>`，再由
   `ActionGrant<A>` 包装返回。permission interaction 是零到多条关联同一 attempt 的 event。
   `PolicyGrantError::Audit` 保留普通 evidence write failure；`IdentityAllocation` 保留 allocation
   failure；记录 allocation failure 也失败时，`IdentityAllocationAndAudit` 同时保留 allocation 与
   audit source。每个 error 携带 core 准备写入的 exact evidence，但不能声称 sink 已经接受它。
   任一失败都必须发生在 grant 逃逸前。`ActionGrant` 不暴露可重组字段；`into_granted()` 是进入
   execution proof 的唯一过渡。
4. **Execution evidence**：runtime-private `ToolInvocationAction::run` 消费真实
   `Granted<ToolInvocationAction>`，在 dispatch 前记录 `Started`，在 dispatch 后记录 `Completed`、
   `InputRejected` 或 `Failed`。窄 Kernel writer 从同一个 proof 读取 grant id/subject/action snapshot，
   caller 不能另传 id 或 persisted evidence envelope。report 已经随 authorization evidence 持久化；
   execution event 不重复 report，`ToolImpl` 也不获得 audit API。invalid caps override 是 grant 前的
   structured invocation rejection，不伪造 policy report 或 governed grant-linked terminal event。
   future 在 Started 后被 drop 时，private guard 尝试写 `OutcomeUnknown`；该 best-effort write 不能
   被描述成 durable terminal evidence。可传播的 cancellation event 等到步骤 14 有真实 cancellation
   owner/caller 时再增加，不先造空 schema。
   generic `Granted<Action>` / Access execution evidence 尚未实现，留给独立后续目标。

`FanoutAuditSink` 只保证 engine 对配置的 sink 发起一次 write 调用。它不提供跨 child sink 的
transaction、rollback 或 retry；某个 child 已接受而后续 child 失败时，error 必须保留这一事实，
engine 不得虚构全局原子提交或自动重试。

attempt 与 generic audit event identity exhaustion 使用独立 typed `AuditError` variant；grant identity
使用 UUID，不再存在 per-Kernel sequence exhaustion。不能退回解析 `AuditError::Sink(String)` 来判断
失败种类。

新 runtime 只写 `AuditEventKind::ActionExecution { grant_id, event }`。action identity、subject、
required capabilities 与 policy report 已由同一 grant 的 authorization evidence 持有，execution event
不重复这些字段。旧 JSONL 继续解码为独立的
`AuditEventKind::ToolInvocation { pack_id, path_display, required_capabilities, outcome }`；该 variant 只表示
不可变历史输入，不是可执行 fallback，也不能由新 runtime 产生。

同一不可变输入规则适用于 `GrantId` representation：新 ID 是 UUID string，历史整数 decode 后仍
必须 serialize 为 JSON number。journal integrity hash 依赖 canonical event serialization；只做到
“旧数字能读”但把它重写成 UUID string 会错误破坏 verify/repair/reopen。protected journal
测试闭合这条兼容边界。

runtime 不得访问 kernel-private audit state。Kernel generic operational recorder 负责普通外部
operational event 的 clock、event id 与 sink write，有真实 ownership 职责，不是 forwarding helper；
它明确拒绝 `AuditEventKind::Authorization`、`AuditEventKind::ActionExecution` 与只读历史
`AuditEventKind::ToolInvocation`，并使用专门的 typed `AuditError` variants，不能退回
`AuditError::Sink(String)` 或字符串分类。typed authorization 只能由 sealed grant algorithm 经 private
backend 写入；typed execution 只能由接收真实 `&Granted<A>` 的窄 writer 写入。writer 从 proof 读取
grant id，authorization snapshot 仍由同一 grant 的先前 evidence 提供；architecture check 把当前
production caller 限定在 runtime invocation owner。这里不增加 active-state map、receipt 或第二套 proof。

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

Invocation cancellation 后续需要 execution lifecycle evidence：client disconnect、explicit cancel 和
runtime shutdown 必须能区分；partial output 不能记录成 completed。cancellation 不回滚已经提交的
side effect。当前 ToolInvocation 对正常返回路径闭合 started/terminal outcome；drop guard 只能尝试
记录 `OutcomeUnknown`，无法把 sink failure 传播给已经消失的 caller。步骤 14 必须用显式 cancellation
owner 取代这条 best-effort 边界。generic Access action 的 completed/failed/cancelled evidence 与
“side effect already happened”语义留给独立后续目标，在此之前不能声称 Access execution evidence
已存在。

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
- cooperative cancellation：停止新 action，记录 tool/invocation cancelled；不改写成 policy deny。
- forced abort after grace timeout：记录 cancellation timeout/forced termination，不能假装正常 cancelled。
- legacy fallback：只记录 legacy evidence，直到对应 tool 迁移；typed tests 不接受宽松双断言。

sealed grant algorithm、authorization evidence、runtime-private registered dispatch 和 grant-bound
ToolInvocation execution recorder 已闭合。仍不能借此声称 generic Access execution audit 已经存在。

## 当前偏差

### Hard constraint 与 terminal consent 尚未分段

- permission decision 当前与 allow/deny 一样立即终止 pipeline；registry 尚未编码“所有 hard deny
  先于 terminal consent”。较早的 permission policy 仍可能跳过后续 typed hard deny。
- production 已注册配置驱动的 mutation consent policy，并把当前唯一的 visibility hard gate 放在它
  之前。未预批准且需要交互的调用通过默认 `PermissionRequestError::Unavailable` fail closed；在阶段
  编码完成前不得把新的 hard constraint 注册到 consent 之后。

### Generic Access execution evidence 尚未实现

- 当前 `Granted<Action>::run(ctx)` 只调用 `Action::run`；它不自动写 execution audit。该调用签名
  已经消费携带 id/info/action 的单一 proof，但 generic execution lifecycle 的唯一 owner 尚未确认。
- 因此 filesystem 和其它 Access action 当前只有各自 policy/grant 结果，没有 generic
  completed/failed/cancelled execution evidence。当前 active goal 不得把 authorization evidence
  误写成 execution evidence。
- correlation carrier 已确定为 private-mint `Granted.id/info`，不再比较 outer-id copy 方案。
  尚待步骤 21 决定的是 generic started/terminal/cancelled state machine 由谁强制、Kernel writer 接受
  哪种 domain-neutral event，以及 compound action + audit error 如何表达。不把 sink 挂到 Context 或
  `Granted`，不增加 `Kernel::grant`，也不借 ToolInvocation wrapper 偷渡 generic Action owner。

### Legacy tool/kernel 路径仍大

- `Kernel` 仍持有 `LegacyToolPlane`、memory/connector/runtime legacy planes 和旧 adapter registration。
- `execute_tool_core` 仍有大量 production/test caller；`ToolCoreRequest` / `ToolCoreOutcome` 仍是
  conversation/session/tool ingress 的主 envelope。
- app static catalog、legacy display alias 和 direct dispatch match 仍与 typed plane metadata 重复。
- app conversation ingress 已先查询 typed registry；只有 `LookupError::NotRegistered` 才构造
  `PreparedLegacyToolInvocation`。outer capability override 在 typed path 保留，legacy-only target
  携带 override 时 fail closed；typed failure 不进入第二次 fallback。
- `read`、`write`、`edit`、`glob.search` 与 `content.search` 的 provider/search/prompt metadata
  已统一从 runtime registration 投影；缺少 registration 会 fail closed。static catalog 只继续描述
  真正未迁移的 legacy tools，由步骤 15 逐个删除。
- `config.import` 整体仍是 legacy tool，由 direct preflight 与 `FilePolicyExtension` 保护
  `input_path` / `output_path`；当前没有 access-backed 半迁移实现。
- `authorize_operation` 和 control-plane legacy allow bootstrap 仍依赖旧 pack/token authorization
  surface。

### Streaming disconnect 不取消执行

- `/v1/chat/completions` streaming 在 detached `tokio::spawn` 中执行完整 turn。
- SSE receiver drop 只使 `sender.send` 失败；provider stream、tool、持久化和 final response 继续运行。
- provider streaming loop 与 retry sleep 没有 Invocation cancellation signal；当前执行也没有
  cancelled finalization contract。
