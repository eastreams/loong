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

- concrete `Runtime`、`Session`、`Context<'a>` 和 `RuntimeContextFactory` 都属于 `loong-runtime`。
  这是一个完整 execution domain，不是由 app concrete type 填空的 generic shell。Runtime 不可 Clone，
  持有 `Kernel<RuntimeContextFactory>`、tool plane、shutdown owner 和私有 Session supervisor；kernel
  仍只是 Runtime 内的 governance authority，不持有 typed tool registry 或 Session。
- `runtime::Handle` 是不带 lifetime 的可 Clone 命令能力。它保存 supervisor 的 typed command sender，
  不保存 `&Runtime`；`Runtime::handle() -> &runtime::Handle` 只借出 Runtime 已拥有的 handle，跨
  `'static` task 时 caller 显式 clone。Runtime shutdown 后旧 handle 可以继续存在，但命令必须返回
  closed，不能延长 Runtime 的运行生命周期。
- `Session` 是不可 Clone 的 live agent state，只存在于唯一的 Session runner。Runtime supervisor 保存
  lifecycle record：session id/generation、typed command sender、status/completion receiver、runner
  `JoinHandle` 和 parent relation；它不复制或共享 Session value。`session::Handle` 不带 lifetime、可 Clone，
  只引用这些 command/observation endpoints；不保存 `Arc<Runtime>`、`Arc<Session>`，也不能构造 Context。
- Runtime 始终是 Session actor 的 lifecycle owner。把 Session value 移入 runner future 不转移 spawn、join、
  reparent 或 shutdown ownership。attached child 只是在同一 supervisor graph 中记录 parent；detach 原子
  清除 relation，不移动 Session 或 `JoinHandle`。parent completion/cancellation 由 Runtime 向 attached
  descendants 传播。
- Session runner 独占普通 command receiver、优先级 control receiver、当前 Invocation 和 mutable history。
  sender/observer 可以 Clone，receiver 不能放进 `Arc<Mutex<_>>`、公开 `drain` 或交给 Tool；mailbox 是传输
  边界，不是 history store、status truth 或 authorization proof。
- `loong-runtime` 定义 `InvocationImpl`，app 为一次具体调用实现
  `ConversationInvocation`。`session::Handle::invoke(I)` 只接受这个 owned、`'static` 的 concrete
  invocation value；runtime 在 private `ErasedInvocation` adapter 中擦除它，不能改成 `Any`、JSON
  envelope、全局 callback bag 或 `Runtime<P>` 泛型传播。
- `Invocation<I>` 是 `session::Handle::invoke(I)` 返回的不可 Clone 调用对象，负责本次 typed event、
  cancellation request 与 terminal result。它不保存 Session authority、history owner 或 Context；
  Drop 只能发送 cancellation，真正 finalize 始终由 Session runner 完成。
- private erasure 只解决 Session mailbox 需要存放不同 concrete invocation 类型的问题。本次 input/state
  由 concrete `I` 的字段持有，event/output/error 由 associated types 表达；runtime 不为跨队列而发明
  `Value`/String 中间格式。Session runner 构造 root Context，并在调用 `I::execute` 外强制包住 lifecycle、
  cancellation 与 audit；concrete implementation 不获得 audit 或 root construction。
- `Context<'a>` 是一个 Invocation 内递归执行作用域的不可变视图。Session runner 私有构造 root
  Context；tool、action、nested model 与 batch sibling 从 parent 派生同类型 child Context。Context
  不是 `Invocation` 的返回对象，也不能取消整个 Session。
- 一个 Context 生命周期内，影响 authority 或 policy 输入的字段不可原地修改。child Context
  只能继承或收窄 authority：`child_caps ⊆ parent_caps`，并继承同一 Invocation 的 identity、
  mode/goal 和 cancellation signal。
- Context 的 `Cow` 字段表达存储 ownership：base 字段是 `Cow::Borrowed`，child 中被收窄的字段
  是 `Cow::Owned`。`PolicyContext::allowed_capabilities()` 对两者都返回
  `Cow::Borrowed(self.effective_capabilities.as_ref())`；accessor 只 reborrow，绝不再次 clone。
- Context 不保存 tool/action payload、`CapabilityToken`、pack/token evidence、Runtime/Session
  Handle、session registry、join owner、`ExecutionPlane` 或 `PlaneTier`。payload 属于 concrete
  Action；Invocation、tool invocation、`AccessCx` 和 concrete Action 类型已经表达执行域。
- Context 只保存本次执行所需的窄 services、session id/generation、固定 history branch/head、
  cancellation observation 与 effective authority。canonical history 内容不是 ambient Context
  权限；Provider 使用 Invocation 已读取的固定投影，需要历史的 Tool 必须通过受治理 Access 按同一
  revision 读取，不能默认读取 latest。
- Session、Runtime supervisor 和持久化 store 都不保存 Context。需要 `'static` 的 Session runner
  拥有真实长期资源，并在自己的 future 内构造借用型 Context；不能把整个 Context 重新 Arc 化。
- concrete 名称固定为 `Context<'a>`，GAT marker 固定为 `RuntimeContextFactory`，两者都由
  `loong-runtime` 直接定义。替换掉的 app Context、factory 和 Session owner 不留 alias、deprecated
  wrapper 或 app-root re-export。
- `ContextFactory` 只是 lifetime 到 concrete context 的 GAT 映射，不构造值，也不持有 policy
  engine 或 runtime。
- app 的 config/repository materializer 只能产出 runtime-owned `SessionSpec`。该类型是经过验证后移交
  live identity、authority 和 service ports 的 ownership boundary，不是旧 Session 的同构 wrapper。
  `LoongConfig`、SQLite repository、prompt/provider state 和整份 legacy `ToolRuntimeConfig` 不进入
  Runtime/Session/Context；typed policy 需要的 session authority 必须拆成有 owner 的具体字段。
- `ToolInvocationContext` 是 generic `ToolInvocation<C>` 对 `C::Cx<'a>` 的窄 requirement trait，只表达
  “从 parent 按给定 capabilities 派生同类型、authority 不扩大的 child”。它不构造 root Context，
  不暴露 Kernel/audit/Runtime，也不进入 `ContextFactory`。即使 canonical Context 与 ToolInvocation
  同属 `loong-runtime`，该 trait 仍用于保持 generic ToolPlane primitive 与 concrete Context 解耦；
  不能把它扩成大 deps trait 或同义 service locator。

## Cancellation

- streaming client disconnect 取消当前 `Invocation`，不取消整个 Session。Session cancel 向下取消
  其活跃 Invocation 与 attached descendants；Runtime shutdown 向下取消全部 Session/Invocation。
- Session runner 在执行 Invocation 时必须继续处理 cancellation、permission reply、user input 与
  shutdown control；不能在同一个 command loop 中直接 `.await` Invocation 而阻塞其依赖的控制消息。
  同一 conversation branch 的 mutating Invocation 默认串行，batch tool 并发不受影响；真正需要并发
  Invocation 时必须显式 fork branch，不能完成后自动 rebase。
- cancellation 是协作式执行边界。Provider、tool orchestration 和长时 access operation 在安全
  点观察同一个 Invocation cancellation signal；强制 task abort 只能是超过 grace period 后的最后手段。
- cancellation 不承诺回滚已经提交的 side effect。取消后不得再 grant/启动新的 action；已经进入
  backend 的操作按其原子性契约完成或失败，已经产生的 authorization evidence 必须保留。
  execution evidence 由对应 grant consumption owner 的 contract 保证；cancellation 不能虚构、抹去
  或跨 domain 推断 execution outcome。
- partial assistant output 不能伪装成 completed reply。Invocation finalization 必须区分
  completed、failed 和 cancelled，并恢复 Session 的可继续状态或记录明确的 terminal lifecycle。

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
- terminal authorization write 成功后，core 才能私有 mint `ActionGrant<A>`。`ActionGrant<A>` 只包装
  一个已经绑定 `GrantId + ActionGrantInfo + action` 的 `Granted<A>`，`into_granted()` 是进入 execution
  proof 的唯一过渡。三个事实不能由 caller 拆开后与另一份真实 grant 重组；`Granted<A>` 也不持有
  sink、clock 或 authority handle。私有 mint 仍必须保证 deny 与 audit failure 路径无法绕过。
  Context、caller、policy 和 Access backend 都不手写 authorization evidence。
- 执行入口消费 `Granted<ConcreteAction>`；不能再为“防伪造 grant”增加 `SessionAuthority`、
  token wrapper 或同义证明类型。backend 可以存在，但不要求统一 backend trait。
- policy 不依赖 runtime concrete Context。它通过小 requirement trait 读取所需字段；config ->
  concrete typed policy registration 属于 app bootstrap。
- `AccessCx`、fs facade 等 concrete view 可以存在，但只能借用 Context/Runtime 中的 source of
  truth，不能拥有独立 capability、policy、audit 或 runtime state。
- 每个 Access domain 自己拥有并 re-export 其 public action、options、output 和 facade API。
  concrete operation 按纵向能力共置；不能把所有 domain action 继续堆进一个大 `action.rs`，也不能
  只是把它机械改成按类型分类的 `action/` 子目录。内部 operation module 保持 private，调用方依赖
  `loong_access::<domain>::{...}`，不依赖内部文件路径或 crate-root flattening。

## Tool

- 普通调用入口必须是 `ctx.tool(path)?.invoke(payload).await`。`Context::tool(path)` 只使用 Session
  runner 在 root Context 中安装的窄 tool execution service；它不能取得或构造 `runtime::Handle`。
  runtime `ToolInvocation` 通过 `ToolInvocationContext` 派生 capability 不扩大的 child，再用
  `PolicyEngine::grant` 完成 grant、internal dispatch 和强制 execution audit。
- runtime tool service 在 grant 前完成唯一一次 registry lookup，并把 concrete `RegisteredTool` entry
  绑定进借用型 handle。registry 只负责 storage、resolve 和 enumeration；runtime
  `ToolInvocation` 消费 `Granted<ToolInvocationAction>` 后直接调用已绑定 entry，不能在 authorization
  之后按 path 再 lookup。`RegisteredTool`、registered descriptor owner 和 `ErasedTool` 全部属于
  `loong-runtime` 并保持 raw invoke crate-private；`loong-core` 只保留 concrete implementer 使用的
  `ToolImpl` 抽象。普通 caller 只能获得受治理的 invocation handle，不能构造或调用 raw registered
  entry。
- `Context::tool` 的 lookup failure 保持为 `LookupError`。`with_capabilities_override` 只在 handle 上
  保存 requested narrowing，不同步校验或返回 error；subset validation 属于随后
  `invoke(...).await` 的 governed attempt，失败由 `ToolInvocationError::CapabilityOverride` 保留 typed
  source 并自动记录 rejection evidence。这样扩权尝试不会在 audit 前消失。
- runtime 将 `ActionGrant<ToolInvocationAction>` 转成 `Granted<ToolInvocationAction>` 后立即进入
  `Granted::run`；runtime-private `Action::run` 在同一个 proof 的生命周期内读取 id/info、写 Started、
  dispatch bound entry，再写 terminal evidence。不存在 outer id copy path 或第二套 execution
  metadata。`ToolImpl` 不获得 audit API。`ToolInvocationError` 覆盖 override rejection、child
  narrowing、grant、dispatch 和 execution audit，并保留 typed source，不能降级成字符串。
- runtime 不得访问 kernel-private sink/clock/id state。Kernel generic operational recorder 拒绝
  engine-owned `Authorization`、grant-bound `ActionExecution` 和只读历史 `ToolInvocation`；runtime
  通过绑定真实 `Granted<ToolInvocationAction>` 的窄 execution recorder 写 Started/terminal
  `ActionExecution` evidence。两条边界都由 Kernel 拥有 clock、event id 与 sink write，并返回 typed
  `AuditError`。不得增加
  `AuthorizedToolInvocation`、`AuditHandle`、route/receipt、`ctx.audit` 或同义 authorization proof。
- `ToolPath` 的 identity、segments 与 wire contract 属于 contracts；某个 concrete tool 使用哪个 path
  属于 ToolPlane registration，不属于 tool descriptor。注册点把 path 与 runtime-private registered
  entry 组合。新增 builtin tool 的目标改动面只有 concrete `ToolImpl` 和一条 registration。
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
- `loong-runtime`：concrete `Runtime`、`Session`、`Context<'a>`、`RuntimeContextFactory`、lifetime-free
  command handles、Session supervisor、`InvocationImpl`/private invocation erasure、tool-plane registry，
  以及执行 Context 所需的最小 typed authority domain。它不依赖 `loong-app`，不解析产品配置或持久化
  记录，也不保留旧 transitional runtime spine。
- `loong-app`：config/persistence materialization、builtin Policy/Tool registration、provider/channel/
  conversation integration、concrete `ConversationInvocation` 和 legacy ingress migration。app 把外部
  状态验证为 runtime-owned `SessionSpec` 后移交 ownership；不定义第二套 Runtime/Session/Context，
  也不 re-export 旧名字。
- `loong-tools`：concrete builtin tool implementations only；不拥有 registry、policy、access
  facade 或 legacy envelope。
