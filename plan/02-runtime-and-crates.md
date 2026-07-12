# plan: Runtime / Context / Crate 收敛

本文件定义 runtime/context 二核心模型，以及 crate 收敛策略。它解释 owner 和依赖方向；
具体实现顺序见 `08-next-steps.md`。

## Runtime / Context Ownership

项目已经由 `loong-runtime::Runtime<C>` 持有 kernel 与 typed tool plane；TUI 的 `App` 仍只是
UI state。CLI、channel、conversation 和 provider host surface 统一持有或借用 owned
`AppContext`，由它共享 `Arc<Runtime<AppContextFactory>>` 并派生 invocation overlay。
剩余工作是把 session metadata 正式并入 session-owned context，而不是再引入另一种执行
context。

目标是二核心模型：

- `Runtime`：主体。所有 agent/session/tool plane/config/policy bootstrap/governance state
  都挂在它下面。
- `Context`：统一执行上下文。每个 session 有自己的 `Context` 状态实例；tool/action/policy
  都通过该 concrete context 类型或它派生出的 facade 观察本次执行状态。

`Runtime` 的 concrete owner 放在 `loong-runtime`，但统一 context 的 concrete 类型仍由 app
定义。`loong-runtime::Runtime<C>` 只通过 `ContextFactory` 泛型认识 context，因此不依赖
`loong-app`；app 使用 `Runtime<AppContextFactory>`。这同时允许 runtime 固定拥有
`Kernel<C>` 与 `ToolPlane<C>`，而不会把 app config/session concrete types 下沉到基础 crate。

目标形状示意。示例类型名用于表达 ownership，不要求最终代码逐字使用这些名字：

```rust
pub struct Runtime<C: ContextFactory> {
    kernel: Kernel<C>,
    tools: ToolPlane<C>,
    sessions: SessionRuntime,
    agents: AgentRuntime,
}

pub struct Context {
    runtime: Arc<Runtime<AppContextFactory>>,
    session: SessionId,
    agent: AgentId,
    allowed_caps: CapabilitySet,
    view: ContextView,
}

impl Context {
    pub fn access(&self) -> AccessCx<'_>;
    pub fn tool(&self, path: ToolPlanePath) -> Result<ToolInvocation<'_>, ToolLookupError>;
    pub fn child_with_caps(&self, allowed_caps: CapabilitySet) -> Self;
}

pub struct ToolInvocation<'ctx> {
    ctx: &'ctx Context,
    resolved: ResolvedToolEntry,
    caps_override: Option<CapabilitySet>,
    trusted_overlay: TrustedInvocationOverlay,
}

impl ToolInvocation<'_> {
    pub fn with_capabilities_override(self, caps: CapabilitySet) -> Result<Self, ToolLookupError>;
    pub fn with_trusted_overlay(self, overlay: TrustedInvocationOverlay) -> Self;
    pub async fn invoke(self, payload: Value) -> Result<Value, ToolError>;
}
```

命名不强制叫 `Runtime` / `Context`，但 ownership 必须一致：

- `Runtime` 直接持有 kernel/governance 所需对象，例如 `Kernel`、tool plane、audit sink
  和 clock。`AppContext` 共享 runtime、pack、token、runtime config，并以不可变字段保存
  effective capabilities、plane/tier、request payload 和 fs root view。
- `Runtime` 持有长期状态和 registries，例如 tool plane、agent/session namespace、
  config snapshot、policy registry bootstrap 结果。
- `Context` 是 session 绑定的统一 execution context。每个 session 有自己的 `Context`
  实例；所有 session 的 context 类型相同。它提供 `ctx.access()` 和
  `ctx.tool(path)?.invoke(payload).await`；它不是裸 kernel reference，也不是 TUI state。
- `AccessCx`、后续可能的 tool invocation facade、fs facade 等都可以是具体类型。它们的
  构造入口来自 `Context`，例如 `ctx.access()`；它们只能借用/引用 `Context` 和 runtime
  内部治理对象，不能成为新的 source-of-truth context。
- `Context` 应该是 cheap-clone 的 owned view：共享 runtime/session state 用 `Arc` 或 id，
  本次 invocation 的 overlay（如 `allowed_caps`、plane/tier、request payload）作为不可变
  字段替换。这样避免“owned ctx 没法复制”和“ref ctx 中途没法覆盖”的两难。
- `ctx.tool(path)` 返回 `Result`，因为 plane-local path 解析、registry lookup 或 tool
  visibility 可能失败。它只返回一个 resolved invocation handle；不做 grant、不 parse payload。
- `ToolInvocation::invoke(payload)` 构造同类型 child context 时继承父 runtime/session/agent
  view，但重新计算 `allowed_caps`：`child_caps = parent_caps ∩ requested_caps`。若
  requested caps 来自 override，override 必须先被证明是 tool default caps 的子集。
- legacy reserved payload 字段不能进入 typed `ToolImpl`。迁移期可以在 app ingress 从 agent
  payload 抽取 trusted evidence，转成 `TrustedInvocationOverlay`，然后把 reserved 字段从
  tool payload 中删除。typed tool 只能通过 context facade/requirement trait 观察 overlay
  带来的访问范围变化。
- ordinary tool/action/policy 只依赖 context requirement trait，不依赖 app concrete context。
  具体 runtime/context 类型由 app/runtime 层定义。

后续迁移策略：

1. session 创建时同时创建自己的 `Context`，并把 session id、agent id、initial effective
   caps、tool namespace view 和 runtime reference 绑定进去。
2. invocation 通过 `AppContext::for_invocation(...)` 派生同类型 child context；child 只能
   收窄 effective capabilities，不能重新签发或放大 authority。
3. session metadata 合并完成后，删除仍由 conversation `SessionContext` 重复保存的同源
   execution state；tool/access/action/policy 始终只观察统一 context 或窄 requirement trait。


## Crate 收敛

截至 2026-07-11，workspace 已经是 15 个 crate，而
`docs/design-docs/core-beliefs.md` 仍写着 “13-crate DAG”。crate 数量和文档都已经过时；
后续 crate 收敛计划以真实 owner 为准，不以旧 DAG 为准。

保留硬边界：

- `loong-kernel`：治理权威。小心不要把 app runtime state 塞回 kernel。
- `loong-access`：副作用边界。即使小也保留，因为它提供 “only access can do side effects”
  的物理边界。
- `loong-tools`：concrete builtin tools。它截至 2026-07-11 体量小，是因为只迁了
  aggregate `ReadTool`，不是因为边界错。
- `loong-contracts` / `loong-core`：继续审边界，尤其是哪些类型是真 contracts、哪些只是
  core behavior trait。不要再把 plane-local path 或 legacy envelope 放到 contracts/core。
- `loong` daemon：交付入口，保留；业务 runtime ownership 不继续堆在 daemon。

优先收敛候选：

1. `loong-cli`：只有 transitional CLI shell spine，workspace 内没有反向依赖。应删除或并回
   `daemon` / `loong-app-protocol` 的 owning boundary。
2. `loong-app-protocol`：截至 2026-07-11 是现有注释称为 Phase 2 的 app-facing protocol
   spine，只被 `daemon` 和 `loong-cli` 使用。若 unified runtime 落地，它应被
   `loong-runtime` 吞掉或并回真正的 app/daemon 边界。
3. `loong-runtime`：不要继续保持 transitional spine。要么破坏性重定义为 unified runtime
   crate，要么删除该名字，避免它占用 runtime 概念却不拥有 runtime。
4. `loong-plugin-sdk`：虽然很小但被 kernel 依赖，语义上反了。kernel 需要的 plugin
   contract 类型应下沉到 `contracts` / `protocol`，SDK 应成为外部 plugin author-facing
   crate，而不是 kernel 的依赖。
5. `protocol` / `bridge-runtime`：先审是否是真 wire/bridge execution boundary。如果只是
   helper 壳，合并；如果是稳定协议或真实 bridge primitive，保留。

crate 收敛原则：

- 不按行数砍 crate；按 owner 和依赖方向砍。
- 转发壳、phase spine、compatibility facade 不能长期存在。
- crate 合并/删除要同步更新 `Cargo.toml`、workspace dependencies、docs DAG、architecture
  boundary checks 和 release/public docs，不能只让代码编译。
- `docs/design-docs/core-beliefs.md` 里 “No breaking changes” 和 “13-crate DAG” 已不符合
  本计划的 refactor 原则；后续应改成“破坏性迁移旧包袱，但必须记录 owner 决策和验证路径”。
