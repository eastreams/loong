# plan: 原则与分层边界

本文件记录长期原则和 crate/module ownership。它不列具体提交步骤；具体步骤见
`08-next-steps.md`。

## 已确认原则

- 敢于破坏性改动。替代边界确认后，直接迁移调用点并删除被替代入口；不保留 alias、
  proxy 或长期 fallback。避免会模糊 ownership 的 root re-export；清晰的 domain module
  re-export 可以接受，例如 `loong_access::fs::{FsAccess, FsReadAction}`，但需保持导出路径唯一。
- concrete unified `Context` 类型由 runtime/app 层定义。kernel/access/policy/tool 只通过
  `ContextFactory` 和小的 context requirement trait 观察它。
- tool/access/policy 共享同一个 session-level `Context` source of truth。`AccessCx` 这类
  从 `Context` 派生的 concrete facade 可以存在；旧 `ToolCoreContext` 是删除目标。
- 副作用 only access can do。已经迁入 Access-Action-Policy 路径的 tool/helper/adapter/kernel
  policy 都不能直接执行文件读取等 migrated side effect。
- `ActionMeta::required_capabilities` 是 action 属性；workspace root、file root、
  runtime config 是 context / resolver / policy 的输入，不塞进 required caps。
- context 暴露的是 effective allowed caps，不一定等于原始 token caps。tool 调 tool 时，
  child context 的 caps 必须从 parent effective caps 缩窄出来。调用参数可以提供
  `Option<required_caps_override>`；有 override 时先校验它是 tool default caps 的子集，
  再用它替代 default caps 计算 child effective caps。没有 override 时使用 default caps。
- `Granted<A>` 是授权到执行的边界。没有 grant 就不能进入对应 side-effect 或 dispatch
  入口。
- backend 可以存在，但不要求统一 trait。硬约束是每个执行入口消费
  `Granted<ConcreteAction>`，例如通过 `Granted<A>::run(ctx)` 进入 action 自己的执行
  hook。
- `read` 可以是 aggregate tool，但不能是 aggregate action。`read { path }`、
  `read { query }`、`read { glob/pattern }` 的泄漏面不同，最终必须落到不同 concrete
  action。
- 路径解析本身是 fs action。canonicalize、existing ancestor resolution、symlink
  resolution 都是 filesystem observation，不能藏在未治理 helper 里。
- 路径权限的共享产物叫 `GrantedPath`。它不是泛型 token，而是 fs domain 的 concrete
  value；只能由受治理的路径解析 action 产出，构造函数不公开。
- workspace root / allowed roots 是 path-resolution policy 的输入，不是
  `FsReadAction` 的运行需求。`FsReadAction` 只应消费已经治理过的 `GrantedPath`。
- policy 不依赖 app concrete context。需要 context 数据时，用小 requirement trait
  表达，例如 fs root view；业务 policy 应为任意满足 trait 的 context 实现。
- config -> policy 路径属于 app bootstrap：app 读取 config，构造 typed policy，注册进
  pipeline。access 不读取 app config，tool helper 不做 policy preflight；config 不能
  通过修改 action required caps 来表达业务授权。
- `path` 是 ToolPlane registry 的路径，不是 contracts/core 的全局概念。具体 path
  类型由具体 `ToolPlane` 定义；core 不能替所有 plane 规定 `ToolPath` 的结构。
- `ToolImpl` 不拥有 path。tool 自身只描述输入/输出/能力/说明；注册到某个 plane 时，
  plane 才把自己的 `Path` 和 tool descriptor 组合成 registered spec。
- `loong_contracts::ToolOutcome` 直接删除。typed target 是 `Result<Value, E>`：success
  是 `serde_json::Value`，failure 是该层自己的 error type。尚未迁入 typed path 的 legacy
  bridge 继续用 `ToolCoreOutcome` 兼容旧 app/tool-core 边界。
- `ToolInvocationAction` 可以存在，但它属于具体 plane/app 的治理边界，不能在 core 里
  持有全局 `ToolPath`。类型名不加 `App` 前缀；层级由模块路径表达，例如
  `loong-app::tools::plane::ToolInvocationAction`。公共泛型 helper 只有在多个 plane
  真的复用同一形状时再引入。
- `ActionMeta::payload` 没有默认 `Null`。payload 是 Action 的结构化载荷；如果 action
  已经持有 `serde_json::Value`，目标 API 可以返回 `Cow<'_, Value>` 来避免无意义 clone。
- 减少 helper function。能用类型、trait bound、owned boundary 或 action/context
  结构表达的约束，不用 helper 暗中搬运或转换。只有在它统一多处真实重复的调用形态、
  且该形态不适合用类型表达时，helper 才可以存在；存在时必须在 helper 附近写清楚理由
  和归属边界。
- 架构代码要有少量高信号注释，标明边界和意图。注释解释 why，不重复代码，也不写大段
  散文。
- 对外不再传播 `KernelContext` 作为 app/runtime owner；最终该类型应删除。host
  surface、conversation、provider、channel、tool orchestration 应拿到统一 runtime 或统一
  context；“持有 kernel 的 capsule”只是 runtime 内部 governance 组件，迁移完成后也不应
  以 `KernelContext` 公共类型存在。
- unified runtime 是 app 运行时 owner：持有 tool plane、session/agent view、runtime config
  snapshot、config -> policy wiring 和每次 invocation context 的构造入口。kernel 不持有
  app tool registry，也不拥有 session/agent/tool namespace。
- 运行时从两个核心派生：一个所有 agent/session 都挂载其上的主体 `Runtime`，加一个统一
  `Context`。每个 session 都有自己的 `Context` 状态实例，所有 session 使用同一个 concrete
  context 类型，类比 Linux `task_struct`。
- 统一的是 source-of-truth execution context，不是禁止具体 facade/view 类型。`AccessCx`
  这种从 `Context` 派生出的 concrete Cx 很有必要；它可以承载 domain API 和窄依赖，但不能
  拥有独立 runtime、capability、policy 或 audit 状态。
- 一次 tool/action invocation 使用同一个 `Context` 类型的本次 invocation view。它持有 runtime
  reference、本次 invocation 绑定的 session/agent、effective allowed caps、request payload、plane/tier 和
  fs root 等小 view。tool->tool 调用必须构造同类型 child context 来缩窄 caps，而不是复用
  父 context 或直接传 token。
- tool invocation 的普通调用入口是 `ctx.tool(path)?.invoke(payload).await`。`ctx.tool(path)`
  返回 `Result<ToolInvocation<'_>, ToolLookupError>`：它只做 plane-local path 解析/entry lookup，
  不做 grant、不 parse payload。返回的 invocation handle 借用 `&ctx`，绑定 resolved entry、
  optional caps override 和 trusted overlay；`invoke(payload)` 才是构造 child context、kernel
  grant、plane dispatch 和 execution audit 的治理边界。
- `trusted_internal_payload` 这类保留字段不是 typed tool payload。legacy ingress 可以临时从
  agent payload 中抽取 trusted evidence，但必须立即转成 typed context overlay，并从传给 tool
  的 payload 中删除。concrete tool 不直接读取 trusted overlay；它只能通过 `ctx.access()`、
  `ctx.tool(path)?.invoke(...)` 或窄 context requirement trait 观察 overlay 的效果。
- legacy tool code 可以在迁移期留存，但不能被包装成新架构组件来假装已迁移。typed
  tool invocation 先尝试 app-owned ToolPlane；未注册/未迁移时只在调用边界末尾 fallback 到
  legacy adapter/core-tool 路径。旧工具不注册进 typed ToolPlane，不通过 payload claim
  混入新 path，也不把旧 policy helper 塞回 `PolicyPipeline` 冒充 typed action policy。
- crate 边界要服务真实 owner。小 crate 不是问题；只做转发、占用大名字但没有 owner 职责、
  或保留 phase 过渡壳的 crate 是问题。确认替代 owner 后应破坏性收敛，不用 alias/fallback
  保留被替代形状。


## 分层边界

- `loong-contracts`：稳定数据类型，例如 `ToolInputError`、`ToolCoreOutcome`、
  `PolicyReport`，以及 kernel/sink 需要的 generic audit primitives。`ToolCoreOutcome` 是
  legacy app/tool-core invocation envelope；`loong_contracts::ToolOutcome` 应删除，而不是
  作为 typed tool compatibility layer 保留。不要定义全局 `ToolPath`；不要让 tool
  descriptor 携带 registry path。
- tool invocation shortcut 挂在 app-defined context 派生的 invocation handle 上，例如
  `ctx.tool(path)?.invoke(payload).await`。`ToolPlane` trait 本身只表达 granted dispatch，
  不接收 kernel/audit 参数，也不从 ctx 暗中偷裸 audit API。
- `loong-core`：行为 trait 和不可伪造授权模型，例如 `ActionMeta`、`Action<Cx>`、
  `Granted<A>`、`ToolImpl<C>`、`RegisteredTool<C>`。core 不决定 ToolPlane path 类型；
  core 里的 registered tool 只擦除 concrete tool，不表达注册位置，也不把 success
  payload 包成 legacy envelope。
- `loong-kernel`：governance authority，提供 policy pipeline、token/pack boundary、
  grant、audit sink、clock/event id。kernel 不持有 typed tool registry，也不拥有
  concrete ToolPlane path 类型。
- `loong-access`：domain side-effect boundary。文件读取只在 fs access/action 路径中
  发生。
- `loong-runtime`：目标是 unified runtime crate，而不是截至 2026-07-11 仍只有
  `loong-core` 依赖的 transitional protocol spine。它应成为持有 app runtime owner 的地方：tool plane、
  session/agent namespace、effective caps context、config -> policy registration wiring、
  以及 invocation context construction。若直接移动会被 app config/provider/channel
  类型强耦合，可以先在 `loong-app` 内 staging；但 `loong-runtime` 不能长期保留为
  只有宏大名字、没有 runtime owner 职责的过渡壳。
- `loong-app`：concrete integration layer。它装配 provider/channel/TUI/config/memory 等
  app 侧适配，调用或构造 unified runtime，但不应继续让各 surface 直接持有
  `KernelContext`。迁移期间保留 legacy fallback orchestration；最终 tool/session/access
  调用都应经 unified runtime/context。
- `loong-tools`：concrete builtin tool implementations only。该 crate 不承载
  `ToolImpl`、`RegisteredTool`、registry、plane、policy/action 抽象；它只放
  `ReadFileTool` 这类具体工具和它们的 input/output 类型及小范围格式化逻辑。concrete
  tool 不直接返回 legacy envelope。
