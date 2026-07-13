# plan: Kernel / Audit / 当前实现偏差

本文件记录 kernel/audit 的稳定职责，以及截至 2026-07-13 仍存在的实现偏差。

## Kernel Boundary

Kernel 是 governance authority：

- 验证 pack/token/revocation/time/capability boundary；
- 运行 typed policy pipeline；
- 发放 `ActionGrant<A>` / `Granted<A>`；
- 持有 audit sink、clock、authorization/event/grant identity；
- 不持有 typed ToolPlane，不 dispatch concrete tool，不拥有 app Session/Context。

tool invocation、filesystem operation 和其它 domain intent 都走同一个 generic action grant
contract。不要增加 `AuthorizedToolInvocation`、tool-specific receipt 或只把 generic grant 包一层的
helper。

现有 `loong_core::kernel::Kernel<C>` trait 只为 Access 暴露 `policy_engine()`，导致 direct Access
绕过 token 复查与 generic authorization audit。目标 contract 应直接暴露最小 generic grant
boundary：Access 依赖 core trait 并持有 Kernel 引用，不能依赖 concrete `loong-kernel`，也不能只
拿裸 `PolicyEngine`。统一 Context 通过 core-owned `ActionAuthorizationContext` 从 Session 借出
`CapabilityToken`；Kernel 从 token 解析 pack id、从自己的 clock 取时间，并结合 child Context 的
effective capabilities 完成检查。contract 返回 core-owned typed `AuthorizationError`，不能暴露
concrete `KernelError`；Access error 用 `thiserror` 透明承载它。不要为此增加第二套宽 forwarding
governance trait。

## Audit Invariant

audit 分为两个强制边界：

1. **Authorization evidence**：Kernel 在 policy evaluation 前分配只用于 audit correlation 的
   authorization attempt id。每个已结束 attempt 记录一条 terminal event；permission
   request/resolution/failure 另记零到多条关联 interaction event。generic grant 记录 action
   metadata、required caps、pack/token 结果、policy report 与 allow/deny；成功 grant 另有 grant
   id。deny 不进入 execution。
2. **Execution evidence**：grant consumption owner 记录 completed/failed/input-error/cancelled。对 tool
   是 `ctx.tool(...).invoke(...)`；对 fs 是 concrete `Granted<Action>::run(ctx)`。

两层不能重复表达同一事实：

- capability/token/policy deny 只属于 authorization evidence；
- tool 内部 fs policy deny 是 fs action authorization deny，同时让外层 tool execution 以 domain
  error 失败；不能伪装成 ToolPlane route/deny；
- concrete tool 不获得裸 audit API；runtime invocation wrapper 自动记录 execution outcome；
- audit 使用 stable display/resource，不把 ToolPlane slot、registry key concrete type、legacy route
  或 fallback 语义固化进 contracts/kernel。

Turn cancellation 另有 execution lifecycle evidence：client disconnect、explicit cancel 和 runtime
shutdown 必须能区分；partial output 不能记录成 completed。已经提交的 side effect 保留各自 action
execution evidence。

## Tool Failure Matrix

- pack/token/capability reject：generic action authorization deny；plane 不执行。
- invocation policy deny：generic action authorization deny，包含 `PolicyReport`；plane 不执行。
- payload parse/input error：invocation grant 已消费，记录 tool execution input error；不 fallback。
- concrete tool error：记录 tool execution failed。
- tool 内部 access deny：记录 domain action authorization deny；外层 tool execution failed。
- cooperative cancellation：停止新 action，记录 tool/turn cancelled；不改写成 policy deny。
- forced abort after grace timeout：记录 cancellation timeout/forced termination，不能假装正常 cancelled。
- legacy fallback：只记录 legacy evidence，直到对应 tool 迁移；typed tests 不接受宽松双断言。

## 当前偏差

### Generic grant evidence 不完整

- `ActionGrantInfo` 已保存完整 allow `PolicyReport`，但 direct Access 仍调用
  `PolicyEngine::grant`，绕过 Kernel 的 token expiry/revocation 复查与 authorization audit。
- `Kernel::grant_action` 对 token/policy deny 仍转换成 legacy `PolicyError` 并记录旧 authorization
  denial；没有统一记录 action metadata、attempt id、report 和 grant id。
- `KernelInvocationContext::request_parameters()` 仍让 legacy `PolicyAny` 从 Context 读取另一份请求
  JSON，而不是读取 `ActionMeta::payload()`。
- permission decision 当前与 allow/deny 一样立即终止 pipeline；registry 尚未编码“hard constraints
  先于 terminal consent”，较早的 permission policy 仍可能跳过后续 typed hard deny。

### Tool execution audit ownership 未收敛

- `contracts::AuditEventKind::ToolInvocation` 仍是 tool-specific event；
  `Kernel::record_tool_invocation` 仍由 kernel 构造它。
- 当前 event 没有 grant id；typed authorization 与 execution evidence 不能可靠关联。
- app/runtime 强制记录 execution outcome 的要求已经明确，但 sink 如何承载 app-owned payload 仍需
  与现有 closed `AuditEventKind` contract 一起收敛，不能只移动函数名。

### Legacy tool/kernel 路径仍大

- `Kernel` 仍持有 `LegacyToolPlane`、memory/connector/runtime legacy planes 和旧 adapter registration。
- `execute_tool_core` 仍有大量 production/test caller；`ToolCoreRequest` / `ToolCoreOutcome` 仍是
  conversation/session/tool ingress 的主 envelope。
- app static catalog、legacy display alias 和 direct dispatch match 仍与 typed plane metadata 重复。
- `authorize_kernel_action`、`policy_engine_error`、`authorize_operation` 和 control-plane legacy allow
  bootstrap 仍依赖旧 `PolicyError` surface。

### Context 仍是旧 owner

- typed tool invocation 已通过 `AppContext::tool(...).invoke(...)` 请求 generic action grant，但
  `AppContext` 仍是 Arc/COW session+invocation 混合体。
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
