# plan: Runtime / Session / Context / Crate 收敛

本文件记录 execution runtime 已确认的边界，以及尚未闭合的 owner/control 候选。当前设计门禁、
执行顺序与完成线见 `08-next-steps.md`；长期 grant、Access、Tool 和 migration 原则见
`01-principles-and-boundaries.md`。

## 当前过渡事实

- typed execution spine 已经成立：runtime tool invocation 走
  `PolicyEngine::grant -> ActionGrant<A> -> Granted<A>`，typed hit 后的 input、grant、dispatch 和 audit
  error 不进入 legacy fallback；migrated filesystem side effect 已进入 Access。
- `loong-contracts::ToolPath` 是唯一 tool identity；`loong-runtime::tool_plane` 已拥有固定使用该 path
  contract 的 registered entry、sealed erasure、一次 lookup 后绑定 entry 的 `ToolInvocation`，以及分阶段
  typed errors。registry 可以替换索引结构，不能替换 identity 类型或 wire 语义。
- 当前 `loong-runtime::Runtime<C>` 只持有 Kernel 与 ToolPlane；app-owned Clone `Session` 与 Runtime 分开
  保存，再由公开 `Context::new(runtime, session)` 现场组合。`RuntimeId`、runtime mismatch 和
  `rebind_session` 都是该 split ownership 的补偿机制，不是目标 API。
- 当前 Context 暴露 crate-visible Runtime，app production 因而能从 Context 访问 generic audit writer 和
  legacy Kernel。narrowed Context 也能重新调用 root constructor 恢复 Session baseline authority。
- detached typed execution 仍从 legacy dispatcher 反向取得 Runtime；因此 legacy owner 目前仍在塑造
  typed lifecycle。
- 旧 `loong-runtime` crate-root `RuntimeSpine`、one-shot/interactive executor 和 task projection 已删除；
  仍有 caller 的 wire/projection 类型已归属 `loong-app-protocol`。后续 owner cutover 不得恢复这套
  transitional spine、alias 或 forwarding adapter。
- 当前 `loong-kernel::mailbox` 仍是 app Session 使用的 legacy notification queue：receiver 被
  `Arc<Mutex<_>>` 包装并由外部 `drain`，消息还携带 `trigger_turn`。它不是目标 Session actor mailbox，
  不能继续扩展；Runtime owner cutover 必须迁移真实 caller 后删除它。

这些事实说明 typed grant path 可复用，但 owner cutover 尚未完成。

## 设计状态：未完成

Runtime、Session 与 execution Context 的最终 owner/control 形状尚未定型。尤其还没有决定：

- Session state 应由单一 runner 独占，还是由受控的共享 state 配合唯一 lifecycle loop；
- host 应以 `invoke(I) -> Invocation<I>` 为主，还是以 `submit(input/control) -> event/status stream` 为主；
- turn snapshot、model-step snapshot 与 recursive Tool/Access authority Context 应如何分层；
- 哪些长期状态必须由 Runtime registry 强拥有，哪些只应通过 control/event endpoint 暴露。

OpenAI Codex 当前的 `ThreadManager -> CodexThread -> Session -> ActiveTurn -> TurnContext ->
StepContext -> ToolInvocation` 形状是重要参考：subagent 是独立 Thread/Session，长期 Session 通过
submission/event/status channel 驱动，tool cancellation 从 active turn 派生。但它不是待照抄的目标；
Codex 的 broad `Arc<Session>` ToolInvocation 与大量共享可变状态不符合 Loong 的窄 Context 和
`Granted<Action>` 边界。

在上述问题形成明确结论前，下面内容只保存先前候选，不能作为可直接实施的 contract。不得以实现
候选步骤的方式替代设计决策，也不得据此引入兼容层。

## 待决候选形状

```text
loong_runtime::Runtime
  owns Kernel<RuntimeContextFactory>
  owns ToolPlaneRegistry<RuntimeContextFactory>
  owns all live Sessions
  -> runtime::Handle
  -> session::Handle
  -> invoke(ConversationInvocation)
  -> Invocation<ConversationInvocation>
  -> private Session runner
       -> private ErasedInvocation adapter
       -> private root Context<'a>
            -> InvocationImpl::execute
            -> recursive child Context<'a>
            -> ctx.tool(path)?.invoke(payload)
            -> ctx.access()...
            -> PolicyEngine::grant
            -> ActionGrant<A>
            -> Granted<A>
            -> ToolPlane / Access
```

`loong-runtime` 是 concrete execution owner，不是等待 app 填入 Context 的 generic shell。
`RuntimeContextFactory` 仍满足 core 的 GAT contract，但 canonical Runtime 本身不把 Context factory 泛型
传播到 host、Session handle 或 Invocation API。

## Runtime

- `Runtime` 不可 Clone，是一个允许存在多个实例、但不实现 global singleton 的长期 owner。
- Runtime 直接拥有已经安装 policy/audit backend 的 `Kernel<RuntimeContextFactory>`、immutable ToolPlane、
  shutdown state 和 private supervisor。builtin/plugin registration 在 supervisor 启动前完成，失败通过
  typed `Result` 返回。
- `runtime::Handle` 不带 lifetime、可 Clone，只保存 typed command endpoint。它不保存
  `&Runtime`、`Arc<Runtime>`、Kernel、ToolPlane 或 Session registry。
- `Runtime::handle() -> &runtime::Handle` 只借出 Runtime 已拥有的 handle；跨 `'static` task 的 caller
  显式 clone。Runtime shutdown 后旧 handle 返回 closed，不能延长 owner lifecycle。
- explicit shutdown 先关闭 spawn/reparent，再取消 Invocation/Session，最后等待全部 Session runner。
  Drop 只能发出 shutdown signal，不能声称 async finalization 已完成。
- 不增加 `RuntimeInner`、`RuntimeShared`、app Runtime facade 或第二个 runtime owner。确需跨 runner 共享的
  Kernel/ToolPlane service 使用其具体 owner/storage，不把所有状态塞进一个大 `Shared`。

## Session

- `Session` 是一个 live agent 的不可 Clone state，只由唯一 Session runner 持有。Runtime supervisor 是
  lifecycle owner，但只保存 session id/generation、command/status endpoints、runner join owner 和 parent
  relation；runner future 独占 Session value，以及其中的 stable identity、effective authority、mailbox、
  selected Access ports 与 invocation state。
- `session::Handle` 不带 lifetime、可 Clone，只提供 invocation 和由 Runtime 仲裁的窄 lifecycle/
  observation command。Monty programmable method 名称与 sync/async surface 在接入时决定；handle 不拥有
  join/finalize，不保存 `Arc<Session>`，也不能构造 Context。
- `session::Handle::invoke(I)` 接受 owned `I: InvocationImpl` 并返回 typed `Invocation<I>`。handle 只把
  private erased job 送入 Session mailbox；它不执行 app algorithm，也不接收 callback/factory。
- Runtime 始终是所有 root/child/detached Session runner 的 lifecycle owner。attached child 只记录 parent
  relation；detach 在同一 supervisor graph 内按 id/generation 原子清除 parent，不移动 runner/Session
  value 或 join ownership，也不建立第二个 registry。
- durable Session identity 与 live Session owner 是两件事。rehydrate 可以复用 durable id，但必须取得新
  generation；旧 handle 和 stale command fail closed。
- subagent 固定为 child Session。普通 nested model invocation 沿用当前 Context，不伪装成 Session，也不
  自动升级成 detached work。

## Session Runner 与 Mailbox

- Runtime supervisor 是唯一 agent graph owner，负责创建、查找、parent/child relation、generation、join、
  shutdown 和状态索引。Session runner 是一个 live agent actor，独占 Session value、history、mailbox
  receiver 和当前 Invocation；Tool 与 Context 都不是 actor。
- 每个 runner 使用 bounded ordinary command channel、独立的高优先级 control channel，以及只读 status/
  completion subscription。普通 backpressure 不能阻塞 interrupt/shutdown；receiver 不 Clone、不公开
  `drain`，mailbox 内容也不作为 durable history。
- spawn/list 属于 supervisor；message/follow-up 经 supervisor 校验 caller/target 后路由到目标 ordinary
  inbox；interrupt 进入 control channel；wait 订阅 status/completion，不向可能卡住的目标 mailbox 发查询。
  supervisor 从当前 Context/Session 派生 caller identity，不能信任 payload 自报 author。
- Invocation completed 只让 Session 回到可继续的 idle 状态，不销毁 agent。follow-up 复用同一 Session；
  interrupt 结束当前 Invocation 而不是关闭 Session。精确的 programmable agent SDK、method 名称和
  sync/async surface 延后到 `08-next-steps.md` 的 Monty goal，本轮只固定 owner 与 transport 语义。

## SessionSpec Boundary

- app 的 config/repository materializer 负责读取 `LoongConfig`、SQLite snapshot、delegate event 和其它
  external input，完成 lineage 与 authority validation 后生成 runtime-owned `SessionSpec`。
- `SessionSpec` 是从 materialized state 向 live Runtime 转移 ownership 的 typed boundary，不是旧 app
  Session 的同构 DTO。Runtime 接受后仍复查 parent generation 与所有 narrowing relation。
- `SessionSpec` 只包含 live execution 真正需要的 identity、parent、capability ceiling、ToolView、fs
  resolution/allowed roots 与 typed Access service ports。mailbox/channel 由 Runtime 创建，不从 app
  materialization 输入搬入。
- 整份 `LoongConfig`、SQLite repository、provider/prompt state、`RuntimeSelfContinuity` 和 legacy
  `ToolRuntimeConfig` 不进入 Runtime/Session/Context。typed Policy 的全局配置由 policy instance 持有；
  session-specific policy input 必须拆成所属 domain 的 concrete field/requirement trait。
- app 的 `ConstrainedSubagentExecution` 可以继续表达 request/event；真正的 child authority 在验证边界
  生成 runtime-owned child Session data。使用 `TryFrom`/typed constructor 表达验证，不增加同构转换
  helper。
- Context 当前依赖的 memory stage/output 若是 Access contract，应迁到 Access/runtime owner；不得让
  `loong-runtime` 为复用 app memory implementation 而依赖 `loong-app`。

## Invocation

- `Invocation<I>` 是 `session::Handle::invoke(I)` 返回的不可 Clone live-call handle，不是 final result，
  也不是内部 Context。它持有 `I` 对应的 typed event/result receiver 与 cancellation request endpoint；
  terminal result 由 `finish()` 消费。
- Session runner 负责 completed/failed/cancelled finalization、history/audit handoff 和 Session 回到可继续
  状态。Invocation Drop 只能请求取消，不能自行写 terminal outcome。
- 本次 owner cutover 建立 Invocation/Session control flow 和 shutdown correctness；provider/gateway/Access
  的完整 cooperative streaming cancellation 仍由 `08-next-steps.md` 的 cancellation goal 完成，不能在
  本轮声称已经端到端支持。

## Invocation Extension Boundary

`loong-runtime` 必须执行 app-owned conversation/provider algorithm，但不能依赖 `loong-app`。这里使用与
typed Tool 相同的“concrete impl + private erasure”边界，而不是 callback/factory：

```rust
#[async_trait]
pub trait InvocationImpl: Send + 'static {
    type Event: Send + 'static;
    type Output: Send + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn execute(
        self,
        ctx: &Context<'_>,
        events: &InvocationEvents<Self::Event>,
    ) -> Result<Self::Output, Self::Error>;
}
```

- `InvocationImpl` 归 `loong-runtime`；它故意不要求 object-safe。app 定义 owned
  `ConversationInvocation`，捕获本次调用真正需要的 normalized input、provider/conversation services
  与 product options，并实现该 trait。
- runtime-private `ErasedInvocation` adapter 保存 concrete value、typed result/event channel 和
  cancellation state，向 Session mailbox 提供唯一的 dyn execution shape。它由 runtime 构造并 sealed；
  app 不能实现或直接调用 raw erasure。
- `session::Handle::invoke(I)` 保持 generic 只到调用点；`Runtime`、`runtime::Handle`、`Session` 与
  `session::Handle` 都不传播 `I`。不同 invocation 的 concrete output/error 通过各自的
  `Invocation<I>` 返回，不进入 `Any`、`Value`、String 或一份 runtime-wide business envelope。
- private Session runner 构造 root Context，并在 `I::execute` 外统一执行 admission、serialization、
  cancellation、audit 与 terminal finalization。`ConversationInvocation` 不获得 Runtime、Kernel、audit
  sink、Session owner 或 root Context constructor。
- 当前 app `ConversationRuntime` 可以继续作为 conversation implementation 内部的小服务 contracts；
  它不能整体注册进 runtime，也不能把每个方法逐个 forward 成新的 runtime trait。app 只实现一个真实
  `ConversationInvocation::execute` 边界，内部调用一次明确的 coordinator entry。
- 不安装 global `Arc<dyn SessionProgram>`：它无法携带每次调用的 owned state，最终会逼出 factory 或
  side table。不使用 `Runtime<P>`：它会把 app behavior 泛型传播到 host/Session/daemon。也不接受调用方
  传裸 future/closure：concrete `InvocationImpl` 是可命名、可注释、可测试的 execution contract。

## Context

- concrete `Context<'a>` 与 `RuntimeContextFactory` 由 `loong-runtime` 定义。Context 是一次 Invocation
  内的 recursive execution scope，不是 Turn、Session、Invocation result 或 host handle。
- root Context 只由 private Session runner 构造。删除 app `Context`、公开 `Context::new`、
  `rebind_session`、`RuntimeId` 与 runtime mismatch；crate 外不存在重建 root authority 的入口。
- Context 只借用窄 Kernel policy service、ToolPlane entry service、Access ports 和当前 Session/Invocation
  authority projection。它不暴露 Runtime、Session owner/Handle、Kernel、audit sink、registry 或 generic
  audit API。
- Context 必须 cheap Clone。base effective capabilities 使用 `Cow::Borrowed`；child 只在取交集后使用
  `Cow::Owned`。所有 child authority 满足 `child_caps <= parent_caps`，accessor 只 reborrow。
- Context 不保存 action/tool payload、pack/token、`ExecutionPlane`、`PlaneTier`、legacy envelope、
  persistence repository、task join owner 或 ambient latest-history reader。
- `ContextFactory` 仍只有 GAT：`type Cx<'a> = Context<'a>`，没有 factory method。value construction 因
  Context 与 Session runner 同属 `loong-runtime` 而保持 private。
- generic runtime ToolInvocation 通过窄 `ToolInvocationContext` requirement 派生 child。concrete
  Context 实现该 trait，但 root constructor 保持 private；trait 不暴露 Runtime/Kernel/audit，不进入
  `ContextFactory`，也不扩成大 deps trait。
- `ctx.tool(path)?.invoke(payload)` 与 `ctx.access()` 是普通 typed execution 的唯一入口。Context 内部可以
  借用真正 service owner，但 caller 不能取得这些 owner 再手工重建 invocation/facade。

## Extension Boundary

- Action/Access/Policy 依赖由各自 owner 定义的小 requirement trait；Policy 对任意满足 trait 的 Context
  实现，不依赖 concrete runtime Context。
- app 通过 concrete `InvocationImpl` 扩展一次 Session 调用；runtime 只传播 private erasure，不让
  app behavior 类型污染 Runtime/Handle。这个 execution extension 与 Context requirement trait 是两类
  边界，不能混成一个大 Context deps trait。
- `ToolImpl<C>` 继续是 core 中 concrete tool implementer 使用的行为 contract；`ToolInvocationContext`
  保持 generic ToolInvocation 对 Context child narrowing 的窄约束；registered erasure 与 raw
  invoke 留在 runtime crate-private boundary。canonical Runtime 只实例化
  `RuntimeContextFactory`，不把 `C` 泛型传播到 host API。
- app 负责构造并注册 concrete Policy/Tool instances。Tool 的 immutable global config 由 tool instance
  持有；需要 session-specific authorization 的数据进入 SessionSpec/domain context trait，而不是 AnyMap
  或一份大 deps/config。

## Legacy Boundary

- `CapabilityToken`、pack、manual authorization、`KernelInvocationContext` 和 `ToolCore*` 不进入 typed
  Runtime/Session/Context/Invocation contract。Runtime 是 Kernel 的唯一 owner，因此 kernel/core target 的
  fallback 由 `loong-runtime` 内明确标注、可机械 allowlist 的 legacy ingress module 执行；app-only
  legacy tool 继续由 app ingress 自己 dispatch。runtime 不反向依赖 app，也不接收 forwarding callback。
- app/daemon 只在 typed registry 返回 `LookupError::NotRegistered` 或进入明确的 unmigrated non-tool
  ingress 时做一次 legacy target routing。typed hit 后任何 error 都不 fallback；两类 legacy owner 都不
  增加同构 request/authorization wrapper，普通 runtime handle API 也不暴露 raw Kernel。
- legacy module 不能拥有、返回或定位 typed Runtime/Session，不能构造 root Context，也不能被
  detached/subagent typed execution 当作 lifecycle owner。它的 imports、caller 和删除条件必须由
  architecture check 精确 allowlist。
- 全部 legacy tool/plane 和 bearer 删除属于后续逐 owner 迁移；本轮只强制依赖方向和唯一 ingress
  containment，不谎称 legacy 已经清零。

## 清理旧 loong-runtime

- crate root 只导出真实 runtime ownership 与 tool-plane domain。已经迁到 `loong-app-protocol` 的
  one-shot/interactive/task projection 不恢复 dependency、alias 或 forwarding adapter。
- 删除旧 `RuntimeSpine`、`RuntimeSurface*`、executor traits、projection helpers，以及迁移后无 caller 的
  public API、error variant、fixture、feature 和 dependency。
- 新 Runtime 替换当前浅层 `Runtime<C>` 后，删除 `RuntimeId`、`id()`、`legacy_kernel()`、generic
  `record_audit_event()` 和接受外部 Context 的 `Runtime::access/tool`。
- 审查 `registered_tool_paths() + tool_metadata()` 两阶段查询。若 caller 只需 enumeration，提供一个直接
  遍历 immutable registered metadata 的 typed API；不保留重复 lookup helper，也不暴露 raw dispatch。
- 测试按 `runtime/`、`session/`、`context/`、`invocation/` 和 `tool_plane/` owner 放置；不恢复大 crate-root
  tests module。
- Cargo dependencies 随 owner migration 精确增删；例如 `RuntimeId` 删除后同时删除仅为它存在的 `uuid`。

## Crate Boundary

- `loong-contracts`：稳定 data/error/report/audit primitives。
- `loong-core`：ContextFactory、Action/Granted/Policy/ToolImpl 等行为 contract。
- `loong-kernel`：capability/policy/grant/audit authority；不执行 typed Tool/Access side effect。
- `loong-access`：domain side-effect physical boundary 与 action-specific requirement traits。
- `loong-runtime`：concrete Runtime/Session/Context/Invocation、handles/supervisor、ToolPlane 和最小 live
  authority domain，以及 `InvocationImpl` 的 private erased runner；不依赖 app。
- `loong-tools`：concrete builtin implementations only。
- `loong-app`：materialization、registration、provider/conversation/channel integration 和 legacy ingress；
  定义 concrete `ConversationInvocation`，不再定义或 re-export Runtime/Session/Context owner。

## 本边界不自动保证

- Context lifetime 不负责 durable identity、跨进程唯一激活或 crash recovery；repository generation 与
  Runtime supervisor 共同拒绝 stale actor。
- cancellation signal 不回滚已执行 side effect，也不代表 provider/gateway 已完成端到端取消接线。
- filesystem path grant 仍不是 inode capability；descriptor-relative TOCTOU closure 是独立目标。
- generic Access execution evidence 尚有独立 owner/design goal；authorization evidence 不能冒充 execution
  evidence。
- Capabilities 的集合表示只在当前 Runtime/Session owner cutover 内暂时保留；值语义 bitset 是已经
  确定的独立后续目标，不再以 benchmark/profile 作为是否迁移的进入条件。
