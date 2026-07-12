# plan: Runtime / Context / Crate 收敛

本文件定义 runtime/session/context 分层，以及 crate 收敛策略。它解释 owner 和依赖方向；
具体实现顺序见 `08-next-steps.md`。

## Runtime / Context Ownership

项目已经由 `loong-runtime::Runtime<C>` 持有 kernel 与 typed tool plane；TUI 的 `App` 仍只是
UI state。当前代码的 `AppContext` / `AppContextInner` / `AppContextFactory` 是迁移期旧形状：
它把 session authority、runtime owner 和 invocation overlay 塞进一个 `Arc`-backed COW
对象。标准 CLI 已经先解析 session 再签发 authority，但 channel/gateway 仍长期持有这个旧
root context。目标不是继续修补旧类型，而是彻底替换为下面固定的命名与 ownership。

目标分层：

- `Runtime`：一个治理域内唯一的 runtime owner，持有 kernel、tool plane 和长期 registries。
- `Session`：一个 agent/task 实例，拥有 session identity、authority 和 lifecycle state；它不
  持有 invocation context，也不把 runtime authority 复制成第二份 capsule。
- `Context<'a>`：Session 在一次 turn/tool/action/policy 执行中的统一借用投影。所有 session
  使用同一个 concrete context 类型，child context 只替换或收窄 execution overlay。

`Runtime` 的 concrete owner 放在 `loong-runtime`，但统一 context 的 concrete 类型仍由 app
定义。`loong-runtime::Runtime<C>` 只通过 `ContextFactory` 泛型认识 context，因此不依赖
`loong-app`；app 使用 `Runtime<RuntimeContextFactory>`。这同时允许 runtime 固定拥有
`Kernel<C>` 与 `ToolPlane<C>`，而不会把 app config/session concrete types 下沉到基础 crate。

目标名称是硬约束，不是示意：

```rust
pub struct Runtime<C: ContextFactory> {
    kernel: Kernel<C>,
    tools: ToolPlane<C>,
}

pub struct Session {
    id: SessionId,
    authority: SessionAuthority,
    state: SessionState,
}

pub struct Context<'a> {
    runtime: &'a Runtime<RuntimeContextFactory>,
    session: &'a Session,
    execution: ExecutionOverlay<'a>,
}

pub struct RuntimeContextFactory;

impl loong_core::policy::context::ContextFactory for RuntimeContextFactory {
    type Cx<'a> = Context<'a>;
}

impl<'session> Context<'session> {
    pub fn access(&self) -> AccessCx<'_>;
    pub fn tool(
        &self,
        path: ToolPlanePath,
    ) -> Result<ToolInvocation<'_, 'session>, ToolLookupError>;
    pub fn child_with_caps(&self, allowed_caps: BTreeSet<Capability>) -> Context<'session>;
}

pub struct ToolInvocation<'ctx, 'session> {
    ctx: &'ctx Context<'session>,
    resolved: ResolvedToolEntry,
    caps_override: Option<BTreeSet<Capability>>,
    trusted_overlay: TrustedInvocationOverlay,
}

impl<'ctx, 'session> ToolInvocation<'ctx, 'session> {
    pub fn with_capabilities_override(
        self,
        caps: BTreeSet<Capability>,
    ) -> Result<Self, ToolLookupError>;
    pub fn with_trusted_overlay(self, overlay: TrustedInvocationOverlay) -> Self;
    pub async fn invoke(self, payload: Value) -> Result<Value, ToolError>;
}
```

Session 的具体 strong owner 仍需结合 detached task 与 structured concurrency 决定；可以是
runtime scope、entry surface 或 supervised task。这个决策不能改变上述边界：Session 不存
`Context`，Context 借用 Runtime + Session，detached task 若需 `'static` 就持有真正的
runtime/session owner，并在 future 内构造 `Context<'_>`。

命名与 ownership 约束：

- `Runtime` 直接持有 kernel/governance 所需对象，例如 `Kernel`、tool plane、audit sink
  和 clock；它不持有 invocation Context。
- `Runtime` 持有长期状态和 registries，例如 tool plane、agent/session namespace、
  config snapshot、policy registry bootstrap 结果。
- `Session` 是 agent/task 的生命周期主体；Context 只借用它需要暴露给本次执行的 authority
  与 view，不拥有 mailbox、task supervisor 或可变 registry。
- `Context<'a>` 是 session 绑定的统一 execution context。它提供 `ctx.access()` 和
  `ctx.tool(path)?.invoke(payload).await`；它不是裸 kernel reference，也不是 TUI state。
- `AccessCx`、后续可能的 tool invocation facade、fs facade 等都可以是具体类型。它们的
  构造入口来自 `Context`，例如 `ctx.access()`；它们只能借用/引用 `Context` 和 runtime
  内部治理对象，不能成为新的 source-of-truth context。
- `Context<'a>` 不使用 `Arc<AppContextInner>`、`DerefMut` 或 `Arc::make_mut`。稳定数据从
  Runtime/Session 借用；只有“通常继承、偶尔覆盖”的 execution overlay 才按字段选择借用、
  `Cow` 或 owned value，不能把 `Arc` 或 `Cow` 铺满整个 Context。
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

1. entry surface 解析或创建具体 session 后，通过 runtime authority 只签发一次该 Session
   的 authority；Session owner 保存它，不能每 turn 重新签发。
2. 明确 Session 的 strong owner 和 cancellation/join/registry removal 语义。lifetime 只约束
   Context 的借用有效性，不替代 Session 的业务 lifecycle。
3. 每次 execution 从 `&Runtime + &Session` 构造 `Context<'a>`；tool->tool/action child Context
   只能收窄 effective capabilities，不能重新签发或放大 authority。
4. 破坏性删除 `AppContext`、`AppContextInner`、`AppContextFactory` 及其 constructors；不得
   保留 type alias、deprecated wrapper、root re-export 或同义 compatibility helper。

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
