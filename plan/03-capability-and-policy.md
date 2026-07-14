# plan: Capability 与 Policy

本文件记录 capability、policy pipeline 和 grant metadata 不变量，以及仍未完成的 contract
收敛。

## Capability Gate

- 每个 concrete action 通过 `ActionMeta::metadata()` 声明 required capabilities。
- `PolicyEngine::grant` 在 policy chain 前执行 capability gate。缺 cap 时不执行 policy，不产生
  `Granted<A>`。
- Context 暴露本次执行的 effective capabilities。base Context 来自 Session baseline 与 Turn
  options 的交集；tool->tool child Context 只能继续缩窄。
- tool caps override 必须先证明 `override ⊆ tool_default_caps`，再计算
  `child_caps = parent_caps ∩ override`。无 override 时使用 tool default caps。
- `ToolInvocationAction` 只授权进入一个 tool；tool 内部 filesystem/network/memory/process
  side effect 仍需各自的 domain action grant。
- `PolicyContext::allowed_capabilities()` 目标签名是
  `Cow<'_, Capabilities>`。base Context 的字段是 `Cow::Borrowed`，child 中被收窄的字段是
  `Cow::Owned`；accessor 对两者都返回
  `Cow::Borrowed(self.effective_capabilities.as_ref())`，绝不为读取再次 clone。
  `Cow` 只优化 recursive Context 的存储借用/收窄，不改变 `child_caps ⊆ parent_caps`。
- `Capabilities` 先使用当前集合表示，避免在本轮同时引入表示层重构。bitset 迁移必须由
  benchmark/profile 证明 capability membership、集合求交或 child narrowing 是 hot path 后再
  单独进行；不得把 bitset 细节泄露进 Policy/Action/Context trait。

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
- `Policy` 与 `PolicyAny` 都通过 `&C::Cx<'_>` 读取 Context，不持有 Factory，不依赖 app concrete
  Context。
- `PolicyPipeline::new()` 是 default deny；legacy allow fallback 必须显式选择，且不能授权新
  typed tool/access action。
- `PolicyEngine::grant` 是唯一 typed authorization API，core 拥有不可由外部 implementor 覆写的
  grant algorithm。`PolicyEngine` 对 caller 只暴露 `grant`；现有 decision/grant-id implementation
  hooks 与 audit write/identity source 一并收进窄 backend contract。需要解耦 kernel 实现时，可以
  在该 contract 上 blanket 实现 `PolicyEngine`；不能把 core algorithm 重新开放成可绕过的
  default method，也不能让 core 依赖 kernel `AuditError`。

## Grant Metadata

`ActionGrantInfo` 已保存发放 grant 所依据的完整 `PolicyReport`。目标 grant boundary 还要求：

- 当前字段形状由 outer `ActionGrant<A>` 提供 `GrantId`、grant metadata 和不可伪造的
  `Granted<A>`；当前 goal 保持 `Granted<A>` 只保存 action，不新增 `grant_id()`，也不复制 outer
  metadata；
- deny 继续通过 typed authorization error 保存 report；
- kernel generic authorization audit 直接使用 allow/deny report，不重新运行 policy，也不生成
  替代 reason；
- `Granted<A>::as_ref()` 只允许 execution boundary 在消费前读取 action metadata，不能提供
  clone/mint/bypass API；`Granted<A>` 不持有 sink、report 或 authority。

`ActionGrant<A>` / `Granted<A>` 的 mint 是 core-private，因此成功 grant 是不可伪造的证明。这个
边界不自动保证 grant algorithm 正确；仍必须让 capability deny、policy deny、permission failure
和 authorization audit failure 都无法到达 mint。不要为同一目的引入 `SessionAuthority`、
authorize token、permit wrapper 或另一层 `Granted`。

`PolicyEngine::grant` 也是 typed authorization evidence 的唯一自动触发点。terminal write 成功后
才能 mint；`PolicyGrantError::Audit` 在 core 通过 source-preserving boundary 保留 concrete sink
error，而不反向依赖 kernel。concrete caller、Context、policy 和 Access backend 不参与 evidence
写入，也不能增加一个 public grant wrapper 代替该 contract。

## Typed 与 Legacy Authorization

- 新 typed path 是
  `Context -> PolicyEngine::grant -> ActionGrant<A> -> Granted<A> -> execution`。Access 和 typed
  tool invocation 不接收 pack/token；不为这条链新增 `Kernel::grant` forwarding method。
- `CapabilityToken`、`KernelInvocationContext`、`authorize_token`、`authorize_operation` 和旧 plane
  execution 可以继续服务仍在运行的 legacy fallback，但只能位于明确的 legacy owner/bounded
  impl；不得成为 typed grant、Access 或 typed ToolPlane authorization 的 trait bound。
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
- fs resolution root、fs allowed roots、provider-specific view 等不放进这个基础 trait；它们由
  对应 domain requirement trait 表达。
- 整个 `KernelInvocationContext` 属于 legacy fallback；它不能作为 `Kernel<C>` 普通 API 的全局
  HRTB。type-erased policy 读取 `ActionMeta::payload()`；typed policy 直接读取 concrete action。

## Config -> Policy

config-driven policy 只在 app/runtime bootstrap 注册：

```text
config -> concrete policy value -> PolicyPipeline registration -> PolicyReport
```

- access 不读取 app config；tool helper 不做 direct policy preflight。
- config 不能通过修改 action required caps 表达 path/filename 等业务授权。
- fs resolution allow、allowed-roots containment、filename deny 和各 concrete operation allow 都是
  typed policy。`deny_read_filenames` 已是普通 config input，不是写死的临时 deny。
- `FilePolicyExtension` 只允许覆盖尚未迁移的 legacy `config.import` skills bridge；不能扩回
  read/write/edit/search 或其它已迁移 action。
