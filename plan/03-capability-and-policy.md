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
- `CapabilityContext::allowed_capabilities()` 目标返回借用，不能为每次 capability gate clone
  整个 `BTreeSet<Capability>`。

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

## Grant Metadata

`PolicyEngine::grant` 已经获得 allow `PolicyReport` 和 `GrantId`，但当前
`ActionGrantInfo` 仍是空 placeholder，allow report 被丢弃。目标：

- `ActionGrantInfo` 保存发放 grant 所依据的完整 `PolicyReport`；
- `ActionGrant<A>` 同时提供 `GrantId`、grant metadata 和不可伪造的 `Granted<A>`；
- deny 继续通过 typed authorization error 保存 report；
- kernel generic authorization audit 直接使用 allow/deny report，不重新运行 policy，也不生成
  替代 reason；
- `Granted<A>::as_ref()` 只允许 execution boundary 在消费前读取 action metadata，不能提供
  clone/mint/bypass API。

## Context Requirement

- `CapabilityContext` 是所有 governed execution Context 的基础要求，因为每个 action 都必须先过
  capability gate。
- fs resolution root、fs allowed roots、provider-specific view 等不放进这个基础 trait；它们由
  对应 domain requirement trait 表达。
- `KernelInvocationContext::request_parameters()` 是 legacy request duplication，应删除。
  type-erased policy 读取 `ActionMeta::payload()`；typed policy 直接读取 concrete action。

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
