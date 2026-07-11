# plan: Capability 与 Policy

本文件只记录 capability 和 policy 的授权语义。tool plane、fs path grant、kernel audit
分别在对应文件中展开。

## Capability 不变量

caps 是硬边界，不是 policy 的附属说明。

- 每个 concrete action 自己声明 `required_capabilities`。`PolicyEngine::grant` 在任何
  policy 执行前先做 capability gate；缺 cap 时不进入 policy chain，也不产生
  `Granted<A>`。
- `PolicyContext::capabilities()` 表示本次 invocation 的 effective allowed caps。app
  顶层 context 可以来自 token；子工具 context 必须来自父 context 的 effective caps。
- tool 调 tool 时，调用参数可以提供 required caps override，但 override 只能缩窄：
  `requested_caps = override.unwrap_or(tool_default_caps)`，且 `override ⊆ tool_default_caps`；
  `child_caps = parent_caps ∩ requested_caps`。
- `ToolInvocationAction` 的 caps 只授权进入一个 tool。tool 内部的文件、网络、内存等
  side effect 仍然要各自构造 domain action，并再次通过对应 access/action policy。
- runtime config 可以影响 policy 实例、tool 可见性、默认 tool required caps 的 bootstrap
  wiring；不能在 tool helper/access helper 中绕过 caps gate。

截至 2026-07-12 的状态：

- typed app-plane invocation 已经从 `ToolSpec.required_capabilities` 构造 child
  effective caps；公开 `tool.invoke` 的外层 `capabilities_override` 也会进入同一
  narrowing 路径。override 绑定在 `ToolInvocation` handle 上，`invoke(payload)`
  是唯一 dispatch 入口。
- legacy direct / adapter 路径在迁移完成前仍可能通过旧
  `required_capabilities_for_request` 计算 caps；新增 typed tool 不应扩展这条旧路径。


## Config -> Policy 路径

config-driven policy 只在 app bootstrap 发生。

目标 config -> policy 路径：

```text
config
  -> app bootstrap / runtime policy builder
  -> concrete typed policy value
  -> PolicyPipeline::push_policy / push_pre_policy / push_fallback_policy
  -> PolicyReport
```

需要从旧路径迁入 typed policy/action path 的 policy：

- fs allowed roots / workspace root containment：typed `FsResolvePathAction` policy。
- filename deny，例如“不许读 clippy.toml”：typed `FsReadAction` policy，来自 app config
  或测试 bootstrap，不写死在 access/tool 里。若该 deny 只是测试用例，它的删除条件是对应
  测试不再需要该 policy fixture。
- `FilePolicyExtension` 的 read 分支：已经迁到 access/action path 后应删除；write/edit /
  config.import 在迁移前只能作为 legacy bridge。
- web/network/memory 等后续 policy：同样由 app bootstrap 从 config 构造 policy，注册到
  pipeline；tool helper 只解析输入和调用 access。
