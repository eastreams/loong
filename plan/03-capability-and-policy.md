# plan: Capability 与 Policy

本文件记录 capability、policy pipeline 和 grant metadata 不变量，以及仍未完成的 contract
收敛。

## Capability Gate

- 每个 concrete action 通过 `ActionMeta::metadata()` 声明 required capabilities。
- `PolicyEngine::grant` 在 policy chain 前执行 capability gate。缺 cap 时不执行 policy，不产生
  `Granted<A>`。
- Context 暴露本次执行的 effective capabilities。base Context 来自 Session baseline 与 Invocation
  options 的交集；tool->tool child Context 只能继续缩窄。
- tool caps override 必须先证明 `override ⊆ tool_default_caps`。该证明发生在 runtime-owned
  `invoke().await` 内，并自动记录 structured rejection；builder 只保存 requested value，不能在 audit
  前同步失败。证明通过后令
  `selected_tool_caps = override.unwrap_or(tool_default_caps)`，计算
  `required_caps = { InvokeTool } ∪ selected_tool_caps`，并用
  `child_caps = parent_caps ∩ required_caps` 派生 child Context。
- `ToolInvocationAction` 必须继续声明完整 `required_caps`，不能改成 child 已有的交集。这样 parent
  缺 capability 时会由 `PolicyEngine::grant` 的正式 capability gate 拒绝并写 authorization
  evidence，而不是在 child 构造阶段提前返回一个没有审计的 narrowing error。
- `ToolInvocationAction` 只授权进入一个 tool；tool 内部 filesystem/network/memory/process
  side effect 仍需各自的 domain action grant。
- 在 bitset 迁移前，`PolicyContext::allowed_capabilities()` 的过渡签名是
  `Cow<'_, Capabilities>`。base Context 的字段是 `Cow::Borrowed`，child 中被收窄的字段是
  `Cow::Owned`；accessor 对两者都返回
  `Cow::Borrowed(self.effective_capabilities.as_ref())`，绝不为读取再次 clone。
  `Cow` 只优化 recursive Context 的存储借用/收窄，不改变 `child_caps ⊆ parent_caps`。
- 当前 `Capabilities(Option<Arc<BTreeSet<Capability>>>)` 只是过渡实现：它用 `Option` 表达空集、用
  `Arc` 补偿集合 clone 成本，把集合语义和存储策略混在了一起。active runtime cutover 完成后必须由
  独立目标将其替换为 contracts-owned `Capabilities(u64)` 值语义 bitset。mask 与 capability 的映射只在
  contracts 内可见；Policy/Action/Context trait 不得暴露 mask 或 bit index。bitset 落地后同时删除仅为
  集合共享存在的 `Cow<Capabilities>`，Context 直接保存并按值传递 effective capabilities。

## PolicyPipeline

当前三段顺序是稳定 contract：

```text
pre PolicyAny -> typed Policy<C, A> -> fallback PolicyAny
```

- `Allow` / `Deny` 终止整个 pipeline。
- `Continue` 进入当前子链下一条 policy。
- `Advance` 跳过当前子链剩余 policy，进入下一子链。
- 没有 terminal decision 时 default deny。
- pipeline registry 保留每个 policy 的 id、注册顺序、注册时间和 source location；
  `PolicyReport` 保留完整 evaluation order、stage、grant 和 outcome。
- `Policy` 与 `PolicyAny` 都通过 `&C::Cx<'_>` 读取 Context，不持有 Factory，不依赖 runtime concrete
  Context。
- public `loong_kernel::policy::PolicyPipelineBuilder<C>` 只负责 policy registration；kernel crate
  root 不 re-export 它。`new()` 是 default deny，legacy allow fallback 必须显式选择，且不能授权新
  typed tool/access action。legacy allow 只注册给 concrete `LegacyKernelAction`，不能根据可伪造的
  `ActionMetadata.kind` 字符串放行。builder 不实现 `PolicyEngine`，也没有可选 audit state。
- Kernel installation 是 registration 与 execution 的唯一转换边界：它把 builder 与 non-optional
  `SharedAuditState` 组合成 private `PolicyPipeline<C>`。installed pipeline 与 Kernel 共用 sink、clock、
  attempt/event/grant identity；不存在可运行的 unbound pipeline，也不提供 silent/no-op audit
  constructor。
- `PolicyEngine::grant` 是唯一 typed authorization API，core 拥有不可由外部 implementor 覆写的
  grant algorithm。`PolicyEngineBackend` 只提供 decision、identity allocation 和 durable evidence
  write；core 在该窄 contract 上 blanket 实现 sealed `PolicyEngine`，且不反向依赖 kernel
  `AuditError`。
- `loong_core::kernel::Kernel::policy_engine()` 返回 opaque `&impl PolicyEngine<C>`。跨 crate caller
  只能调用 `grant`，不能借 concrete installed pipeline 调用 backend hooks 或绕过 core algorithm。

## Authorization Evidence

`AuthorizationEvidence` 只由 sealed core grant algorithm 构造，结构固定为
`subject + action + attempt`：

- `AuthorizationAttempt::StartFailed` 只表示 attempt id allocation 失败，不能同时携带 id、report、
  permission 或 terminal outcome；成功分配后才使用 `Started { id, event }`。
- `AuthorizationAttemptEvent::CapabilityDenied` 发生在 policy 前，因此不携带伪造的空 report；policy
  已运行时使用 `Policy { report, event }`，其中 event 只能是 permission interaction 或 terminal
  outcome。
- permission interaction 使用 `Requested`、`Approved`、`Denied`、`EscalatedToUser` 和 `Failed`
  分支表达合法状态；不能退回通用 `Resolved { authority, resolution }` 重新允许
  `User + Escalate`。
- 这个嵌套 sum type 是 contract，不得退回独立 optional `kind/report/id` 字段，也不能让 caller 自由
  组合不可能状态。
- `PolicyGrantError::Audit` 表示已有有效 attempt 时 evidence write 失败；
  `IdentityAllocation` 表示 attempt/grant identity allocation 失败且 failure evidence 已写入；
  `IdentityAllocationAndAudit` 表示 allocation 与记录该失败同时失败。compound variant 必须分别保留
  allocation 与 audit source；所有 variant 都保留 core 准备写入的 exact evidence，但不借此声称 sink
  已接受。

## Grant Metadata

`ActionGrantInfo` 保存发放 grant 所依据的完整 `PolicyReport`。当前 grant boundary 要求：

- core 在同一次 private mint 中把 `GrantId`、`ActionGrantInfo { report, subject, action }` 与 action
  写入 `Granted<A>`；`ActionGrant<A>` 只包装这一个 proof，不再公开可重组的平行字段；
- `ActionGrant::into_granted()` 是唯一 execution transition。`Granted::id()` / `info()` 只读暴露与
  当前 action 同时 mint 的 correlation 与 authorization snapshot，不能替换、clone grant 或重新 mint；
- deny 继续通过 typed authorization error 保存 report；
- kernel generic authorization audit 直接使用 allow/deny report，不重新运行 policy，也不生成
  替代 reason；
- `Granted<A>::as_ref()` 只允许 execution boundary 在消费前读取 action metadata，不能提供
  clone/mint/bypass API；`Granted<A>` 可以携带 immutable grant info，但不持有 sink、clock 或
  authority handle。

`ActionGrant<A>` / `Granted<A>` 的 mint 是 core-private，因此成功 grant 是不可伪造的证明。这个
边界不自动保证 grant algorithm 正确；仍必须让 capability deny、policy deny、permission failure
和 authorization audit failure 都无法到达 mint。不要为同一目的引入 `SessionAuthority`、
authorize token、permit wrapper 或另一层 `Granted`。

`GrantId` 是跨 Kernel 实例与进程重启稳定关联的 UUID，不是每个 Kernel 从 1 重启的 sequence。
新 evidence 只写 UUID。历史整数 ID 不只是“可以 decode”：decode 后必须继续以原 JSON number
representation 序列化，不能被改写成 UUID string。protected journal 的 integrity hash 基于事件的
canonical serialization；改变旧 ID representation 会把合法历史记录误判为篡改。因此迁移测试必须
覆盖 decode -> verify -> repair -> reverify -> reopen/append，并证明历史数字在整条链中保持数字。

`PolicyEngine::grant` 也是 typed authorization evidence 的唯一自动触发点。terminal allow write 成功
后才能 mint；evidence/identity 错误按上一节保留 source，而 core 不反向依赖 kernel `AuditError`。
concrete caller、Context、policy 和 Access backend 不参与 evidence 写入，也不能增加一个 public
grant wrapper 代替该 contract。

## Typed 与 Legacy Authorization

- 新 typed path 是
  `Context -> PolicyEngine::grant -> ActionGrant<A> -> Granted<A> -> execution`。Access 和 typed
  tool invocation 不接收 pack/token；不为这条链新增 `Kernel::grant` forwarding method。
- `CapabilityToken`、`authorize_token`、`authorize_operation` 和旧 plane execution 可以继续服务仍在
  运行的 legacy fallback，但只能位于明确的 legacy owner/bounded impl；源码已不存在
  `KernelInvocationContext`，不得重新用同类全局 context bound 塑造 typed grant、Access 或 typed
  ToolPlane authorization。
- 不把 legacy API 包装成新的 authority abstraction。legacy caller 留在旧路径，新 caller 直接
  使用 typed grant；迁移一个 caller 时删除该 caller 的 token/pack 参数。
- capability collection 是 policy input，`Granted<A>` 是 policy 通过后才能获得的 execution
  proof。审查 Context 的可信构造与 grant 的唯一 production 入口，不能把两者误称为
  `Granted<A>` 可伪造。

## Context Requirement

- `PolicyContext` 是所有 governed execution Context 的基础要求，因为每个 action 都必须先过
  capability gate。
- `PolicyContext` 只读提供 owned typed authorization subject/identity；它不提供 sink、clock、id
  source 或 `ctx.audit`。
- typed Context 使用 `AuthorizationScope::Session { session_id }`；尚未迁移到 Session owner、仍由
  bearer token 驱动的入口必须显式使用
  `AuthorizationScope::LegacyToken { boundary, pack_id, token_id }`，不能伪造 session id，也不能让
  token/pack 字段进入普通 typed Context scope。
- fs resolution root、fs allowed roots、provider-specific view 等不放进这个基础 trait；它们由
  对应 domain requirement trait 表达。
- legacy bearer 所需的局部 context requirement 只能写在对应 fallback method/owner 上，不能成为
  `Kernel<C>` 普通 API 的全局 HRTB。type-erased policy 读取 `ActionMeta::payload()`；typed policy
  直接读取 concrete action。

## Config -> Policy

config-driven policy 只在 app/runtime bootstrap 注册：

```text
config -> concrete policy value -> PolicyPipeline registration -> PolicyReport
```

- access 不读取 app config；tool helper 不做 direct policy preflight。
- config 不能通过修改 action required caps 表达 path/filename 等业务授权。
- fs resolution allow、allowed-roots containment、filename deny 和各 concrete operation allow 都是
  typed policy。`deny_read_filenames` 已是普通 config input，不是写死的临时 deny。
- `config.import` 当前整体留在 legacy path；`FilePolicyExtension` 只为它的 `input_path` / `output_path`
  保留 legacy root authorization。不存在 access-backed 的半迁移 bridge；完成整条 typed Tool +
  Access 迁移后直接删除该 extension，不能扩回 read/write/edit/search 或其它已迁移 action。
