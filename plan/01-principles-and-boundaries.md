# plan: 原则与分层边界

本文件只记录长期不变量和 owner。剩余提交顺序见 `08-next-steps.md`。

## 迁移纪律

- 敢于破坏性改动。替代边界确认后，迁移调用点并删除旧入口；不保留 alias、proxy 或同义
  wrapper。仍有 production caller 的 legacy fallback 可以暂留，但必须有明确 owner/删除条件，
  且不能塑造或进入新 typed contract。
- 每个提交只完成一个可解释的边界变化。机械迁移、行为变化、文档清理和无关重构不能混在
  同一个提交里。
- helper 默认不成立。只有它统一多个真实重复调用面，而且该约束不适合由类型、trait、owned
  boundary 或 action/context 表达时才保留；附近必须用短注释说明理由和 owner。
- 架构注释解释 ownership、why 和安全边界，不复述代码，也不把讨论历史写成注释墙。
- feature flag 控制完整模块或工具族是否存在。feature 关闭时不编译该模块，不在模块内部保留
  disabled stub。
- root re-export 只用于不会混淆 owner 的明确 domain API。每个公开概念保持唯一推荐路径。

## Runtime、Session 与 Context

- `Runtime<C>` 是长期 runtime owner，持有 `Kernel<C>` 和 typed `ToolPlane<C>`。kernel 是
  Runtime 内的 governance authority，不是第二套 app runtime，也不持有 typed tool registry。
- `Session` 是跨 Turn 存活的主体，只拥有 typed identity、lineage、baseline capabilities、稳定
  config 和 lifecycle state。它不保存 `CapabilityToken`、pack 或其它 token evidence；Session 的
  业务生命周期不由 Rust lifetime 表达，Session 也不是 Future。
- `Context<'a>` 是递归执行作用域的不可变视图，不等同于整个 Turn，也不固定代表单次
  invocation。Turn boundary 根据 Session typed baseline、typed options 和 cancellation signal 构造
  base Context；nested invocation 从 parent 派生同类型 child Context。
- 一个 Context 生命周期内，影响 authority 或 policy 输入的字段不可原地修改。child Context
  只能继承或收窄 authority：`child_caps ⊆ parent_caps`，并继承同一 Turn 的 identity、
  mode/goal 和 cancellation signal。
- Context 的 `Cow` 字段表达存储 ownership：base 字段是 `Cow::Borrowed`，child 中被收窄的字段
  是 `Cow::Owned`。`PolicyContext::allowed_capabilities()` 对两者都返回
  `Cow::Borrowed(self.effective_capabilities.as_ref())`；accessor 只 reborrow，绝不再次 clone。
- Context 不保存 tool payload、action payload、`CapabilityToken`、pack/token evidence、
  `ExecutionPlane` 或 `PlaneTier`。payload 属于 concrete Action；tool invocation、`AccessCx` 和
  concrete Action 类型已经表达执行域。
- Session、Runtime registry 和持久化 store 都不保存 invocation Context。需要 `'static` 的任务
  持有真正的长期 owner，并在 future 内构造借用型 Context；不能把整个 Context 重新 Arc 化。
- concrete 名称固定为 `Context<'a>`，GAT marker 固定为 `RuntimeContextFactory`。旧
  `AppContext`、`AppContextInner`、`AppContextFactory` 必须彻底删除，不留 alias、deprecated
  wrapper 或 re-export。
- `ContextFactory` 只是 lifetime 到 concrete context 的 GAT 映射，不构造值，也不持有 policy
  engine 或 runtime。
- `loong-runtime` 拥有窄 requirement trait `ToolInvocationContext<C>`，这是 runtime-owned
  `ToolInvocation` 与 app-defined Context 的唯一直接 contract。它只按给定 `Capabilities` 从
  parent 派生同一 `C::Cx<'_>` child，并返回 typed narrowing error；不暴露 kernel、audit 或
  Runtime，不放进 `ContextFactory`，也不构造 base Context。app concrete Context 直接实现它。
  该 trait 用于跨 crate 表达 child authority narrowing，不是搬运同构数据的 helper。

## Cancellation

- streaming client disconnect 取消当前 Turn execution，不取消整个 Session。Session shutdown
  可以向下取消其活跃 Turn；Runtime shutdown 可以向下取消活跃 Session/Turn。
- cancellation 是协作式执行边界。Provider、tool orchestration 和长时 access operation 在安全
  点观察同一个 Turn cancellation signal；强制 task abort 只能是超过 grace period 后的最后手段。
- cancellation 不承诺回滚已经提交的 side effect。取消后不得再 grant/启动新的 action；已经进入
  backend 的操作按其原子性契约完成或失败，并保留 audit evidence。
- partial assistant output 不能伪装成 completed reply。Turn finalization 必须区分 completed、failed
  和 cancelled，并恢复 Session 的可继续状态或记录明确的 terminal lifecycle。

## Access、Action 与 Policy

- 副作用 only Access can do。migrated tool/helper/adapter/policy/kernel 不能直接执行文件、网络、
  process 或其它 domain side effect。nested typed tool 只能继续编排；它最终仍必须进入一个
  `Granted<ConcreteAction>` 的 Access operation 才能产生物理副作用。
- `ActionMeta::required_capabilities` 是 action 属性。workspace root、allowed roots、runtime config
  等是 context/resolver/policy 输入，不塞进 caps。
- `ActionMeta::payload()` 必须显式返回 `Cow<'_, Value>`，没有默认 `Null`。Action 自身表达
  policy 含义，payload 只是 type-erased structured view。
- `PolicyEngine::grant` 先做 capability gate，再运行 policy pipeline。没有 terminal allow 就
  default deny；它继续是 production typed action 的 grant API，不为隐藏现有 engine 再增加
  Kernel forwarding method。
- `PolicyEngine::grant` 的 implementor 承担自动 authorization audit。concrete kernel
  `PolicyPipeline` 使用 kernel-private shared audit state 取得 sink、clock 与 identity；audit write
  失败必须在 grant 逃逸前成为 typed grant error。Context、caller、policy 和 Access backend 都
  不手写 authorization evidence。
- `Granted<A>` 的构造私有，因此已经是授权到执行的不可伪造边界。执行入口消费
  `Granted<ConcreteAction>`；不能再为“防伪造 grant”增加 `SessionAuthority`、token wrapper 或
  同义证明类型。backend 可以存在，但不要求统一 backend trait。
- policy 不依赖 app concrete Context。它通过小 requirement trait 读取所需字段；config ->
  concrete typed policy registration 属于 app bootstrap。
- `AccessCx`、fs facade 等 concrete view 可以存在，但只能借用 Context/Runtime 中的 source of
  truth，不能拥有独立 capability、policy、audit 或 runtime state。

## Tool

- 普通调用入口是 `ctx.tool(path)?.invoke(payload).await`。`Context::tool(path)` 是薄入口，只调用
  Runtime 创建 handle；runtime `ToolInvocation` 通过 `ToolInvocationContext<C>` 派生 child，再用
  现有 `PolicyEngine::grant` 完成 grant、internal dispatch 和强制 execution audit。
- raw ToolPlane granted dispatch 是 `loong-runtime` 内部 primitive，只消费
  `Granted<InvocationAction>` 并 dispatch；它不能被普通 caller 直接调用，也不知道 kernel
  token、pack、audit sink 或 event id。`ErasedTool` 保持 private/sealed，concrete `ToolImpl` 不得
  绕过 runtime invocation wrapper。
- runtime `ToolInvocation` wrapper 强制记录带 grant id 的 execution outcome；`ToolImpl` 不获得
  audit API。lookup、caps override、child narrowing、grant 和 dispatch 都通过
  `ToolInvocationError` 保留 typed source，不能降级成 `KernelError` 或字符串。
- `path` 属于具体 ToolPlane，不属于 contracts/core。tool descriptor 不带 path；注册点把
  plane path 与 `RegisteredTool` 组合。新增 builtin tool 的目标改动面只有 concrete
  `ToolImpl` 和一条 registration。
- 一个 path 命中后由 concrete tool parse payload。不存在 `ToolPayloadMatch`、payload claim 或
  “解析失败就 fallback”。aggregate `read` 可以内部选择 file/query/glob，但这些分支必须进入
  不同 concrete fs actions。
- typed success 是 `Result<Value, E>`。`ToolCoreRequest` / `ToolCoreOutcome` 只属于未迁移 legacy
  envelope，不能进入 concrete tool crate 或 typed plane。
- legacy tool code 可以暂时存在，但不能注册成 typed tool 冒充迁移完成。typed path 未注册时，
  fallback 只能位于旧 ingress 的最后边界，并随调用面迁移删除。

## Filesystem

- path resolution 是 fs action，因为 canonicalize、existing-ancestor 和 symlink resolution 都是
  filesystem observation。
- resolve action 只产出不可伪造的 resolved fact；path policy 接受 resolved fact 并产出
  `GrantedPath` / `GrantedEntryPath`；具体 fs action 只接受对应 grant type。
- resolution root 是 access/action execution 输入；allowed roots 是 path policy 输入；两者由不同
  context requirement trait 暴露。`FsReadAction` 不读取 workspace root。
- target-following 与 final-component no-follow 必须由 typestate 区分。path grant 目前不等于
  inode/object capability；descriptor-relative backend 落地前必须明确记录 TOCTOU 残余风险。

## Crate Ownership

- `loong-contracts`：稳定数据、error/report/audit primitives；不定义全局 ToolPath，不承载
  concrete ToolPlane registry key。
- `loong-core`：`ContextFactory`、Action/Granted/Policy/ToolImpl 等行为 contract 和不可伪造授权。
- `loong-kernel`：typed capability/policy/grant/audit authority；不执行 typed tool。pack/token 与
  manual authorization 只属于仍在运行的 legacy fallback，不能进入新的 Kernel/Access/Tool
  contract。
- `loong-access`：domain side-effect boundary。
- `loong-runtime`：`Runtime<C>`、ToolPlane primitive 和 plane-local default registry。crate root
  仍有 transitional spine 时应删除旧 spine，而不是删除已经形成的 runtime owner。
- `loong-app`：concrete `Context<'a>`、`RuntimeContextFactory`、Session/Turn option 装配、builtin
  registration、provider/channel/conversation integration 和 legacy ingress migration。
- `loong-tools`：concrete builtin tool implementations only；不拥有 registry、policy、access
  facade 或 legacy envelope。
