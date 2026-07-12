# plan: 原则与分层边界

本文件只记录长期不变量和 owner。剩余提交顺序见 `08-next-steps.md`。

## 迁移纪律

- 敢于破坏性改动。替代边界确认后，迁移调用点并删除旧入口；不保留 alias、proxy、长期
  fallback 或同义 wrapper。
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
- `Session` 是跨 Turn 存活的主体，拥有 session identity、基础 authority 和 lifecycle state。
  Session 的业务生命周期不由 Rust lifetime 表达；Session 不是 Future。
- `Context<'a>` 是一次 Turn 的不可变执行快照，由 app/runtime 层根据 Session authority、
  本次 Turn 的 typed options 和本次执行的 cancellation signal 构造。Plan、Implementation、
  Goal 等模式/选择，以及会影响工具、权限、workspace 或 policy 的选项，都必须在构造时
  归一化。
- 一个 Context 生命周期内，影响 authority 或 policy 输入的 Turn 选项不可原地修改。需要改变
  这些选项时，结束旧执行并构造新 Context。是否仍属于同一个持久化用户 Turn，由 conversation
  层决定。
- nested tool invocation 可以派生同一种 concrete `Context<'a>`，但只能继承或收窄 authority：
  `child_caps ⊆ parent_caps`。它继承同一 Turn 的 identity、mode/goal 和 cancellation signal。
- Context 不保存 tool payload、action payload、`ExecutionPlane` 或 `PlaneTier`。payload 属于
  concrete Action；tool invocation、`AccessCx` 和 concrete Action 类型已经表达执行域。
- Session、Runtime registry 和持久化 store 都不保存 invocation Context。需要 `'static` 的任务
  持有真正的长期 owner，并在 future 内构造借用型 Context；不能把整个 Context 重新 Arc 化。
- concrete 名称固定为 `Context<'a>`，GAT marker 固定为 `RuntimeContextFactory`。旧
  `AppContext`、`AppContextInner`、`AppContextFactory` 必须彻底删除，不留 alias、deprecated
  wrapper 或 re-export。
- `ContextFactory` 只是 lifetime 到 concrete context 的 GAT 映射，不构造值，也不持有 policy
  engine 或 runtime。

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

- 副作用 only access can do。migrated tool/helper/adapter/policy/kernel 不能直接执行文件、网络、
  process 或其它 domain side effect。
- `ActionMeta::required_capabilities` 是 action 属性。workspace root、allowed roots、runtime config
  等是 context/resolver/policy 输入，不塞进 caps。
- `ActionMeta::payload()` 必须显式返回 `Cow<'_, Value>`，没有默认 `Null`。Action 自身表达
  policy 含义，payload 只是 type-erased structured view。
- `PolicyEngine::grant` 先做 capability gate，再运行 policy pipeline。没有 terminal allow 就
  default deny；没有 grant 不能进入 dispatch 或 side-effect execution。
- `Granted<A>` 是授权到执行的不可伪造边界。执行入口消费 `Granted<ConcreteAction>`；backend
  可以存在，但不要求统一 backend trait。
- policy 不依赖 app concrete Context。它通过小 requirement trait 读取所需字段；config ->
  concrete typed policy registration 属于 app bootstrap。
- `AccessCx`、fs facade 等 concrete view 可以存在，但只能借用 Context/Runtime 中的 source of
  truth，不能拥有独立 capability、policy、audit 或 runtime state。

## Tool

- 普通调用入口是 `ctx.tool(path)?.invoke(payload).await`。lookup 只解析 plane-local path 和
  entry；`invoke` 才计算 child caps、构造 `ToolInvocationAction`、请求 grant、dispatch 并强制
  记录 grant 后 execution outcome。
- `ToolPlane::invoke` 只消费 `Granted<InvocationAction>` 并 dispatch；它不知道 kernel token、
  pack、audit sink 或 event id。`ErasedTool` 保持 private/sealed，concrete `ToolImpl` 不得绕过
  plane wrapper。
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
- `loong-kernel`：pack/token/capability/policy/grant/audit authority；不执行 typed tool。
- `loong-access`：domain side-effect boundary。
- `loong-runtime`：`Runtime<C>`、ToolPlane primitive 和 plane-local default registry。crate root
  仍有 transitional spine 时应删除旧 spine，而不是删除已经形成的 runtime owner。
- `loong-app`：concrete `Context<'a>`、`RuntimeContextFactory`、Session/Turn option 装配、builtin
  registration、provider/channel/conversation integration 和 legacy ingress migration。
- `loong-tools`：concrete builtin tool implementations only；不拥有 registry、policy、access
  facade 或 legacy envelope。
