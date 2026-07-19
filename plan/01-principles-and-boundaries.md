# plan: 原则与分层边界

本文件只记录长期不变量和 owner。剩余提交顺序见 `08-next-steps.md`。

## 迁移纪律

- 敢于破坏性改动。替代边界确认后，迁移调用点并删除旧入口；不保留 alias、proxy 或同义
  wrapper。legacy fallback 只允许服务已确认的 production caller，且必须有明确 owner/删除条件，
  不能塑造或进入新 typed contract。
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
- `loong-runtime` 拥有窄 requirement trait `ToolInvocationContext`，这是 runtime-owned
  `ToolInvocation` 与 app-defined Context 的唯一直接 contract。它只按给定 `Capabilities` 从
  parent 派生同类型 child，`derive_tool_child` 返回 `Result<Self, CapabilityNarrowingError>`；不暴露
  kernel、audit 或 Runtime，不放进 `ContextFactory`，也不构造 base Context。app concrete Context
  直接实现它。该 trait 用于跨 crate 表达 child authority narrowing，不是搬运同构数据的 helper。

## Cancellation

- streaming client disconnect 取消当前 Turn execution，不取消整个 Session。Session shutdown
  可以向下取消其活跃 Turn；Runtime shutdown 可以向下取消活跃 Session/Turn。
- cancellation 是协作式执行边界。Provider、tool orchestration 和长时 access operation 在安全
  点观察同一个 Turn cancellation signal；强制 task abort 只能是超过 grace period 后的最后手段。
- cancellation 不承诺回滚已经提交的 side effect。取消后不得再 grant/启动新的 action；已经进入
  backend 的操作按其原子性契约完成或失败，已经产生的 authorization evidence 必须保留。
  execution evidence 由对应 grant consumption owner 的 contract 保证；cancellation 不能虚构、抹去
  或跨 domain 推断 execution outcome。
- partial assistant output 不能伪装成 completed reply。Turn finalization 必须区分 completed、failed
  和 cancelled，并恢复 Session 的可继续状态或记录明确的 terminal lifecycle。

## Access、Action 与 Policy

- 副作用 only Access can do。tool/helper/adapter/policy/kernel 不能直接执行文件、网络、
  process 或其它 domain side effect。nested typed tool 只能继续编排；它最终仍必须进入一个
  `Granted<ConcreteAction>` 的 Access operation 才能产生物理副作用。
- `ActionMeta::required_capabilities` 是 action 属性。workspace root、allowed roots、runtime config
  等是 context/resolver/policy 输入，不塞进 caps。
- `ActionMeta::payload()` 必须显式返回 `Cow<'_, Value>`，没有默认 `Null`。Action 自身表达
  policy 含义，payload 只是 type-erased structured view。
- `PolicyEngine::grant` 是唯一 production typed grant API。capability gate、policy evaluation、
  mandatory authorization audit 和 mint 顺序由 core 固定，对外不可覆写；`PolicyEngine` 对 caller
  只暴露 `grant`，decision、audit write 与 identity source 属于窄 backend contract。可以在该
  contract 上 blanket 实现 `PolicyEngine`，但 core 不能反向依赖 kernel `AuditError`。不为隐藏
  engine 再增加 Kernel forwarding method。
- `PolicyContext` 只读提供 effective capabilities 和 owned typed authorization subject/identity；
  它不暴露 sink、clock、id source 或 `ctx.audit`。
- public `loong_kernel::policy::PolicyPipelineBuilder` 只注册 policy，kernel crate root 不 re-export
  它；Kernel 把 builder 和 non-optional kernel-private audit state 安装成 private
  `PolicyPipeline`。installed pipeline 与 Kernel 共用 sink、clock 和 identity source，不存在可运行的
  unbound pipeline。
- policy-bearing Kernel 不提供 silent/no-op audit constructor 或 sink。测试如果不写 journal，也使用
  可观察的 in-memory sink；自定义 `AuditSink` 是调用方显式选择的 durability trust boundary。
- authorization evidence 用 sum type 编码合法阶段：attempt allocation failure 不能携带 attempt id、
  policy report 或 terminal outcome；capability deny 不能伪装成 policy evaluation；policy 运行后的
  permission/terminal event 必须携带同一份 report。permission resolution 分成 approved、denied 和
  parent-to-user escalation，不能构造 `User + Escalate` 这种非法组合。
- `PolicyGrantError::{Audit, IdentityAllocation, IdentityAllocationAndAudit}` 分别保留 evidence write、
  identity allocation，以及 allocation + failure-evidence write 的双重失败。错误必须携带 core 当时准备
  写入的 evidence 和原始 typed source，并在 grant 逃逸前传播；evidence 字段不声称 sink 已经接受它。
- terminal authorization write 成功后，core 才能私有 mint `ActionGrant<A>` 与 `Granted<A>`。
  `ActionGrant<A>` 承载 grant metadata，`Granted<A>` 是不可伪造的 action execution proof；不把
  sink、report 或 authority 塞进 execution proof。私有 mint 仍必须保证 deny 与 audit failure
  路径无法绕过。Context、caller、policy 和 Access backend 都不手写 authorization evidence。
- 执行入口消费 `Granted<ConcreteAction>`；不能再为“防伪造 grant”增加 `SessionAuthority`、
  token wrapper 或同义证明类型。backend 可以存在，但不要求统一 backend trait。
- policy 不依赖 app concrete Context。它通过小 requirement trait 读取所需字段；config ->
  concrete typed policy registration 属于 app bootstrap。
- `AccessCx`、fs facade 等 concrete view 可以存在，但只能借用 Context/Runtime 中的 source of
  truth，不能拥有独立 capability、policy、audit 或 runtime state。

## Tool

- 普通调用入口必须是 `ctx.tool(path)?.invoke(payload).await`。`Context::tool(path)` 是薄入口，只调用
  Runtime 创建 handle；runtime `ToolInvocation` 通过 `ToolInvocationContext` 派生 child，再用
  `PolicyEngine::grant` 完成 grant、internal dispatch 和强制 execution audit。
- raw ToolPlane granted dispatch 是 `loong-runtime` 内部 primitive，只消费
  `Granted<InvocationAction>` 并 dispatch；它不能被普通 caller 直接调用，也不知道 kernel
  token、pack、audit sink 或 event id。`ErasedTool` 保持 private/sealed，concrete `ToolImpl` 不得
  绕过 runtime invocation wrapper。
- runtime `ToolInvocation` wrapper 必须保留 outer `ActionGrant` metadata，直到关联的 execution
  audit 完成；`ToolImpl` 不获得 audit API。lookup、caps override、child narrowing、grant、dispatch
  和 audit failure 都必须通过 `ToolInvocationError` 保留 typed source，不能降级成字符串。
- runtime 不得访问 kernel-private audit state。跨 crate 写入只通过 Kernel 的 generic
  `record_audit_event` governance recorder；该边界拥有 clock、event id 与 sink write，并向 runtime
  返回 typed `AuditError` source。它不是 forwarding helper；不得增加 `AuditHandle`、route/receipt、
  `ctx.audit` 或同义 capability。
- `path` 属于具体 ToolPlane，不属于 contracts/core。tool descriptor 不带 path；注册点把
  plane path 与 `RegisteredTool` 组合。新增 builtin tool 的目标改动面只有 concrete
  `ToolImpl` 和一条 registration。
- 一个 path 命中后由 concrete tool parse payload。不存在 `ToolPayloadMatch`、payload claim 或
  “解析失败就 fallback”。aggregate `read` 可以内部选择 file/query/glob，但这些分支必须进入
  不同 concrete fs actions。
- typed success 是 `Result<Value, E>`。legacy envelope 类型不能进入 concrete tool crate 或 typed
  plane；fallback 只能位于 legacy ingress 的最后边界，并随对应调用面迁移删除。

## Filesystem

- path resolution 是 fs action，因为 canonicalize、existing-ancestor 和 symlink resolution 都是
  filesystem observation。
- resolve action 只产出不可伪造的 resolved fact；path policy 接受 resolved fact 并产出
  `GrantedPath` / `GrantedEntryPath`；具体 fs action 只接受对应 grant type。
- resolution root 是 access/action execution 输入；allowed roots 是 path policy 输入；两者由不同
  context requirement trait 暴露。`FsReadAction` 不读取 workspace root。
- target-following 与 final-component no-follow 必须由 typestate 区分。path grant 不等于
  inode/object capability；descriptor-relative backend 落地前必须明确记录 TOCTOU 残余风险。

## Crate Ownership

- `loong-contracts`：稳定数据、error/report/audit primitives，以及跨 registration、policy、audit
  和 host 使用的唯一 `ToolPath` identity；不承载 concrete ToolPlane registry/storage。
- `loong-core`：`ContextFactory`、Action/Granted/Policy/ToolImpl 等行为 contract 和不可伪造授权。
- `loong-kernel`：typed capability/policy/grant/audit authority；不执行 typed tool。pack/token 与
  manual authorization 只属于仍在运行的 legacy fallback，不能进入新的 Kernel/Access/Tool
  contract。
- `loong-access`：domain side-effect boundary。
- `loong-runtime`：`Runtime<C>`、ToolPlane primitive 和 default registry；不得保留与
  `Runtime<C>` 并行的第二套 runtime spine。
- `loong-app`：concrete `Context<'a>`、`RuntimeContextFactory`、Session/Turn option 装配、builtin
  registration、provider/channel/conversation integration 和 legacy ingress migration。
- `loong-tools`：concrete builtin tool implementations only；不拥有 registry、policy、access
  facade 或 legacy envelope。
