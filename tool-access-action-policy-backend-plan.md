# Tool Access / Action / Policy Plan

这是临时迁移计划，不并入 `docs/`。当前路线以 Access-Action-Policy 为主，不再让旧
authorization / adapter workaround 塑形新架构。

## 已确认原则

- 敢于破坏性改动。新边界确认后，直接迁移调用点并删除旧入口；不保留 alias、
  proxy 或长期 fallback。避免会模糊 ownership 的 root re-export；清晰的 domain module
  re-export 可以接受，例如 `loong_access::fs::{FsAccess, FsReadAction}`，但需保持导出路径唯一。
- concrete unified context 由 app 定义。kernel/access/policy/tool 只通过
  `ContextFactory` 和小的 context requirement trait 观察它。
- tool/access/policy 共享同一个 invocation context。旧 `ToolCoreContext` 是删除目标。
- 副作用 only access can do。已迁移 tool/helper/adapter/kernel policy 都不能直接执行
  文件读取等 migrated side effect。
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
  是 `serde_json::Value`，failure 是该层自己的 error type。尚未迁完的 legacy bridge
  继续用 `ToolCoreOutcome` 兼容旧 app/tool-core 边界。
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

## 分层边界

- `loong-contracts`：稳定数据类型，例如 `ToolInputError`、`ToolCoreOutcome`、
  `PolicyReport`，以及 kernel/sink 需要的 generic audit primitives。`ToolCoreOutcome` 是
  legacy app/tool-core invocation envelope；`loong_contracts::ToolOutcome` 应删除，而不是
  作为 typed tool compatibility layer 保留。不要定义全局 `ToolPath`；不要让 tool
  descriptor 携带 registry path。
- tool invocation shortcut 挂在 app-defined context 上，例如 `ctx.invoke_tool(path,
  payload)`。它内部使用 app plane registry 构造 invocation action，走 kernel grant，再
  调用 plane 的 `invoke`；`ToolPlane` trait 本身不接收 kernel/audit 参数，也不从
  ctx 暗中偷裸 audit API。
- `loong-core`：行为 trait 和不可伪造授权模型，例如 `ActionMeta`、`Action<Cx>`、
  `Granted<A>`、`ToolImpl<C>`、`RegisteredTool<C>`。core 不决定 ToolPlane path 类型；
  core 里的 registered tool 只擦除 concrete tool，不表达注册位置，也不把 success
  payload 包成 legacy envelope。
- `loong-kernel`：governance authority，提供 policy pipeline、token/pack boundary、
  grant、audit sink、clock/event id。kernel 不持有 typed tool registry，也不拥有
  concrete ToolPlane path 类型。
- `loong-access`：domain side-effect boundary。文件读取只在 fs access/action 路径中
  发生。
- `loong-app`：concrete context、config -> policy wiring、`tools::plane` owns `ToolPlane`、
  concrete tool registration、legacy fallback orchestration。
- `loong-tools`：concrete builtin tool implementations only。这个 crate 不承载
  `ToolImpl`、`RegisteredTool`、registry、plane、policy/action 抽象；它只放
  `ReadFileTool` 这类具体工具和它们的 input/output 类型及小范围格式化逻辑。concrete
  tool 不直接返回 legacy envelope。

## Capability 不变量

caps 是硬边界，不是 policy 的附属说明。

- 每个 concrete action 自己声明 `required_capabilities`。`PolicyEngine::grant` 在任何
  policy 执行前先做 capability gate；缺 cap 时不进入 policy chain，也不产生
  `Granted<A>`。
- `PolicyContext::capabilities()` 表示当前 invocation 的 effective allowed caps。app
  顶层 context 可以来自 token；子工具 context 必须来自父 context 的 effective caps。
- tool 调 tool 时，调用参数可以提供 required caps override，但 override 只能缩窄：
  `requested_caps = override.unwrap_or(tool_default_caps)`，且 `override ⊆ tool_default_caps`；
  `child_caps = parent_caps ∩ requested_caps`。
- `ToolInvocationAction` 的 caps 只授权进入一个 tool。tool 内部的文件、网络、内存等
  side effect 仍然要各自构造 domain action，并再次通过对应 access/action policy。
- runtime config 可以影响 policy 实例、tool 可见性、默认 tool required caps 的 bootstrap
  wiring；不能在 tool helper/access helper 中临时跳过 caps gate。

待补缺口：

- 统一 context 中的 effective caps 仍基本来自 token，尚未实现 tool->tool child context
  的 cap narrowing。
- `ToolImpl` descriptor/path 解耦后，tool default caps 应来自 descriptor，调用 override
  只在 plane 构造 child context 时生效。

## Config -> Policy 路径

config-driven policy 只在 app bootstrap 发生。

目标路径：

```text
config
  -> app bootstrap / runtime policy builder
  -> concrete typed policy value
  -> PolicyPipeline::push_policy / push_pre_policy / push_fallback_policy
  -> PolicyReport
```

当前需要逐步迁移的 policy：

- fs allowed roots / workspace root containment：typed `FsResolvePathAction` policy。
- 临时 filename deny，例如“不许读 clippy.toml”：typed `FsReadAction` policy，来自 app
  config 或测试 bootstrap，不写死在 access/tool 里。
- `FilePolicyExtension` 的 read 分支：已经迁到 access/action path 后应删除；write/edit /
  config.import 在迁移前只能作为 legacy bridge。
- web/network/memory 等后续 policy：同样由 app bootstrap 从 config 构造 policy，注册到
  pipeline；tool helper 只解析输入和调用 access。

## 后续变更约束

这些不是状态清单，而是后续每个最小提交都要继续遵守的约束。

1. Cargo workspace dependency hygiene：
   - 新增或迁移内部 Loong crate 依赖时，先在根 `Cargo.toml` 的
     `[workspace.dependencies]` 加一条统一声明；
   - 叶子 crate 使用 `loong-*.workspace = true`，不要重复写
     `package/version/path`；
   - 历史 daemon/spec/bridge 依赖不要混进功能提交里大扫除，除非该提交正好触碰这些
     crate。

2. Concrete builtin tools crate hygiene：
   - `crates/tools` 只放具体 builtin tools；
   - 不在 `crates/tools` 定义或 re-export `ToolPlane`、`ToolImpl`、`RegisteredTool`、
     `Policy`、`AccessCx` 这类抽象；
   - app feature 只负责开关 concrete tool feature，例如
     `tool-file = ["loong-tools/file"]`；
   - feature flag 控制模块/工具族是否存在，不在模块内部保留 disabled stub；
   - 如果 `file` feature 关闭，`loong_tools::file` 就不应暴露；
   - 模块内部默认 feature 已启用，disabled/unavailable 错误由上层注册或路由处理；
   - concrete tool 需要 access 时，只约束 kernel 暴露的 context requirement，例如
     `for<'a> C::Cx<'a>: KernelAccess<C> + FsAccessContext`。

3. Context access requirement hygiene：
   - `KernelAccess<C>` 这类 trait 是 concrete tools 获取 `ctx.access()` 的窄边界；
   - 它属于 kernel 公共边界，因为返回的是 kernel-defined `AccessCx`；
   - 它不放 `loong-access`，避免 app/spec/test 为实现 context requirement 反向依赖
     access；
   - 它也不放 `loong-core`，因为 core 不该知道 kernel access facade。

4. Grant inspection hygiene：
   - `Granted<A>` 可以提供只读 `as_ref()`，用于 audit 在消费 grant 前读取 action
     metadata；
   - 不能提供从外部构造或复制 grant 的 API；
   - 执行入口仍然消费 `Granted<A>`，例如 `Granted<A>::run(ctx)` 或
     `ToolPlane::invoke(Granted<ToolInvocationAction>, &ctx)`。

5. `ActionMeta::payload` borrowing hygiene：
   - 保持 `ActionMeta::payload(&self) -> Cow<'_, Value>`；
   - 不提供默认 `Null`，每个 action 都必须显式声明自己的 type-erased payload；
   - 对天然可借用的 action，返回 `Cow::Borrowed(&self.payload)`；
   - 对需要临时构造 JSON view 的 action，返回 `Cow::Owned(json!(...))`；
   - 后续 action 迁移不应复制旧的 owned `Value` 签名。

6. Helper function hygiene：
   - 默认先问“这里能不能用类型表达”：例如 concrete action、context requirement trait、
     `Granted<A>`、`GrantedPath`、plane-local path/action、`ToolRegistration`；
   - 不为 legacy envelope、display alias、policy preflight、access construction、payload
     claim 之类边界残留新增 helper。先把转换留在 owning boundary；如果该 boundary
     本身应该消失，就在计划里迁移/删除，而不是扩写 helper；
   - 可以接受的 helper 必须满足两个条件：统一多处真实重复的调用形态；该调用形态不适合
     用类型、trait 或 owned struct 表达；
   - 每个保留的 helper 附近都要有短注释，说明它为什么不是类型、为什么放在这个模块、
     它是否是迁移期边界。没有这些理由，就内联或类型化替代；
   - code review 时发现 helper 只是在搬运同构数据、包装 `from`/`into`、隐藏 policy/access
     边界或制造 alias，应当直接删除并把逻辑放回 owning layer。

7. Comment audit hygiene：
   - 架构边界变更必须补少量注释，说明 ownership 和 why。注释不是“解释代码在做什么”，
     而是把只有迁移作者知道的设计约束写给后续 maintainer；
   - 每次碰下面文件时都要检查注释是否仍然准确：
     - `crates/tools/src/lib.rs`：说明 `loong-tools` 只放 concrete builtin tool
       implementations，不放 `ToolImpl` / registry / policy / access 抽象。这里必须讲清
       “为什么有一个 tools crate 却不承载 tool 抽象”，避免后来者把 plane 或 trait 又搬进来。
     - `crates/tools/src/file.rs`：说明 `ReadFileTool` 只负责 payload parse、调用
       `ctx.access().fs().read_file(...)`、格式化 response；文件读取副作用发生在
       `loong_access::fs`，不能在 tool helper 里直接 `std::fs::read`。
     - `crates/kernel/src/access.rs`：`KernelAccess<C>` 的注释必须讲清它为什么在 kernel
       而不是 core/access：它返回 kernel-defined `AccessCx`，并给 concrete tools 一个
       不依赖 `AppExecutionContext` 的窄 context requirement。
     - `crates/app/src/context.rs`：`AppExecutionContext::access()` / `KernelAccess` impl
       附近要讲清 `AccessCx::new(...)` 只应出现在 concrete context 的 `access()` 实现里；
       普通 tool/action 调用点应使用 `ctx.access()`，不要恢复 `kernel.access(ctx)`。
     - `crates/app/src/tools/plane.rs`：注释必须讲清 `loong-app::tools::plane`
       owns typed plane，kernel 不持有 typed registry；plane 按 path resolve，不按 payload claim；
       `invoke` 消费 `Granted<ToolInvocationAction>`，所以它是 granted primitive；普通调用
       点应使用 `ctx.invoke_tool(...)`，不要绕过 grant shortcut 直接执行。
     - `crates/app/src/tools/mod.rs`：typed dispatch 边界附近要讲清它只是迁移期
       orchestration：`ctx.invoke_tool` -> app plane resolve/build action -> kernel grant ->
       plane `invoke`；`read` query/glob legacy bridge 是临时桥，不是 payload-claim
       设计。
     - `crates/kernel/src/kernel.rs`：generic `grant` 注释必须讲清 kernel 是 governance
       authority，不执行 typed tool；grant 过程负责 action authorization audit；tool
       invocation grant 只授权进入 `ToolImpl`，tool 内部 side effect 仍需自己的 access
       action grant。
     - `crates/contracts/src/audit_types.rs`：逐步只保留 kernel/sink 需要的 generic audit
       primitives。tool-specific execution audit 是 app runtime schema，不应在
       contracts/kernel 固化 `ToolInvocationRoute`、`ToolInvocationOutcome` 或 concrete
       ToolPlane registry key 类型。
     - `crates/loong-core/src/policy/action.rs`：`ActionMeta::payload()` 注释必须讲清 payload
       是 Action 的 type-erased structured view，不是 legacy bridge；签名是
       `Cow<'_, Value>`，并且没有默认 `Null`。
     - `crates/loong-core/src/policy/grant.rs`：`Granted<A>::as_ref()` 注释必须讲清它只用于
       grant 被消费前的 audit/metadata inspection，不能成为伪造、复制或绕过执行边界的入口。
     - `crates/app/src/tools/routing.rs`：legacy read bridge 注释必须讲清 query/glob 暂未迁入
       aggregate `ReadTool`；迁移完成后删除 bridge，而不是把它提升成长期 routing 机制。
   - 注释验收标准：读者只看相关类型/函数附近的注释，就能回答“这个层拥有谁”“为什么不在
     另一个 crate”“这个 fallback 是否长期存在”“谁可以做副作用”“grant 何时被消费”；
   - typed path 测试断言 generic action grant audit + grant 后 tool execution audit；
     legacy path 测试只断言 legacy audit；
   - 不新增 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 模块测试继续放对应模块下，例如 `tools/plane/tests.rs`、`file/tests.rs`。

## Tool Path

`path` 这个命名保留，但类型归属改掉。

错误形状：

```rust
// contracts/core globally decide every plane's path model.
pub struct ToolPath(String);

pub struct ToolSpec {
    pub path: ToolPath,
    // ...
}
```

目标形状：

```rust
slotmap::new_key_type! {
    struct ToolSlot;
}

trait ToolPlane<C: ContextFactory> {
    type Path: Clone + Ord;
}

struct ToolRegistry<C> {
    entries: slotmap::SlotMap<ToolSlot, ToolEntry<C>>,
    paths: BTreeMap<ToolPath, ToolSlot>,
}

struct ToolEntry<C> {
    tool: RegisteredTool<C>,
    registration: ToolRegistration,
}

struct ToolRegistration {
    provenance: ToolProvenance,
}
```

这里的 `ToolPath` 是 `loong-app::tools::plane` 内的 plane-local path，可以是
`Vec<String>`、smallvec、interned path、trie key，或后续其它形状。层级由模块路径表达，
不靠类型名前缀表达；也不是 contracts/core 的决定。

`ToolSlot` 只是 `ToolPlane` 内部注册句柄。外部调用、audit payload、kernel grant、
contracts/core 都不暴露 slot；它们只看 plane 提供的 path display / action payload。
`paths` 负责把 registry path resolve 到 slot，`entries` 承载 `RegisteredTool` 本体、
provenance 和注册元数据。未来如果 path index 换成 trie，只替换 `paths` 这一层，
不用改 entry storage 或 concrete tool。
`ToolEntry` 不保存 path，避免和 `paths` index 形成可 drift 的重复状态。当前不设计
slot-based unregister；如果后续需要，再补 reverse index 或明确 owner invariant。

tool 自身返回无 path descriptor：

```rust
pub struct ToolDescriptor {
    pub description: String,
    pub required_capabilities: BTreeSet<Capability>,
}
```

如果 catalog / agent prompt 需要“path + descriptor”的视图，由 plane 在列举时从
`paths` index 和 entry descriptor 临时投影出来；不要先固定一个 core-level
`RegisteredToolSpec<P>`。

因此新增 tool 的 path 只出现在注册点：

```rust
tool_plane.register(tool_path(["read"]), ReadTool);
```

具体 `Path` 需要能给 policy/audit 提供稳定显示值，但这是通过 `ActionMeta` /
plane-provided formatting 暴露，不是通过全局 `ToolPath` 类型泄漏。

## Tool Invocation Action

Tool dispatch 也是 action，但它不是 `FsReadAction` 这种 domain side-effect action。
它只授权 app orchestration 进入一个 `ToolImpl`：

```text
App execute_kernel_tool_request
  -> canonicalize request
  -> ctx.invoke_tool(path, payload)
  -> AppToolPlane internal resolve/build action
  -> loong-app::tools::plane::ToolInvocationAction(path, required_caps, payload)
  -> Kernel::grant(action)
  -> Granted<ToolInvocationAction>
  -> ToolPlane.invoke(Granted<ToolInvocationAction>, &ctx)
  -> ToolImpl::execute(&ctx, input)
  -> ctx.access().fs().read_file(...)
  -> FsResolvePathAction
  -> PolicyPipeline::grant(ctx, FsResolvePathAction)
  -> Granted<FsResolvePathAction>::run(ctx)
  -> GrantedPath
  -> FsReadAction
  -> PolicyPipeline::grant(ctx, FsReadAction)
  -> Granted<FsReadAction>::run(ctx)
  -> filesystem side effect
```

这意味着 tool invocation policy 和 fs read policy 是两层不同授权：

- `ToolInvocationAction`：允许调用 app plane 上某个 path 的 tool。
- `FsResolvePathAction`：允许把 raw path 解析成 `GrantedPath`。
- `FsReadAction`：允许读取某个 `GrantedPath`。

不能用 `AuthorizedToolInvocation` 这类 receipt workaround 表达这个关系；应该返回
`ActionGrant<ToolInvocationAction>` / `Granted<ToolInvocationAction>`。

本轮迁移不在 core 保留这个 action。`ToolInvocationAction` 放在
`crates/app/src/tools/plane.rs` 附近，使用 app plane 自己的 path 类型和 stable display。
如果后续需要在 core 提供公共 helper，只能是泛型：

```rust
pub struct ToolInvocationAction<P> {
    path: P,
    required_capabilities: Vec<Capability>,
    payload: Value,
}
```

但这不是当前步骤的目标。当前目标是 app/plane 定义 concrete action。这样 policy 可以
通过 `ActionMeta` 观察它，kernel 可以 grant 它，core 不需要知道 path 类型。

## Filesystem Path Grants

“得到一个可供 fs action 使用的路径”是独立 action，不是 read action 的构造细节。
当前形状分三步：

1. `FsAccess` 使用 `FsAccessContext` 的 `fs_resolution_root()` /
   `fs_allowed_roots()` 准备 resolved path facts。这一步需要 canonicalize、existing
   ancestor resolution、symlink resolution，因此属于 access 边界的 filesystem
   observation。
2. `PolicyPipeline` 对 `FsResolvePathAction` 做 typed policy 决策。allowed roots /
   path escape 由 kernel policy deny，denial 进入 `PolicyReport`。
3. 只有 granted resolve action 的 `run` 能 mint `GrantedPath`。下游 read/search/glob
   action 只能接收 `GrantedPath`，不能接收 raw path 或普通 `PathBuf`。

核心类型：

```rust
pub struct FsResolvePathAction {
    raw_path: PathBuf,
    resolved_path: PathBuf,
    allowed_roots: Vec<PathBuf>,
}

pub struct GrantedPath {
    path: PathBuf,
}

impl GrantedPath {
    pub fn as_path(&self) -> &Path;
    pub fn into_path_buf(self) -> PathBuf;
    // no public from/pathbuf constructor
}
```

调用链：

```rust
let resolve = FsResolvePathAction::resolve(
    raw_path,
    ctx.fs_resolution_root(),
    ctx.fs_allowed_roots(),
)?;
let grant = policy_engine.grant(ctx, resolve).await?;
let path = grant.granted.run(ctx).await?;

let read = FsReadAction::new(path);
let grant = policy_engine.grant(ctx, read).await?;
grant.granted.run(ctx).await
```

这里有两个不同授权点：

- `FsResolvePathAction`：允许在当前 context 下把 raw path 解析成 `GrantedPath`。
  action 携带 access 准备好的 resolved path facts；policy 只基于这些 facts 表达
  workspace root、file root、path escape、symlink escape 等路径权限。
- `FsReadAction` / `FsContentSearchAction` / `FsGlobAction`：允许对一个已经治理过的
  `GrantedPath` 执行具体读取、内容搜索、路径枚举。它们仍然各自声明 capability 和
  payload，因为三者泄漏面不同。

`FsResolvePathAction::run` 不再重新 canonicalize，也不读取文件内容。它只消费
`Granted<FsResolvePathAction>` 并把 policy 已接受的 resolved facts 变成 `GrantedPath`。
如果解析结果逃逸 allowed roots，kernel typed policy 会 deny，因而不会产出
`GrantedPath`。

不要写 `FsReadAction::new(Granted<FsResolvePathAction>, ctx)` 这种隐藏执行的 API；
先显式 `grant.granted.run(ctx).await?`，再把 `GrantedPath` 交给下游 action。

## ToolPlane

`ToolPlane` 属于 `loong-app::tools::plane` runtime，不属于 kernel。

目标 plane 形状：

```rust
trait ToolPlane<C: ContextFactory> {
    type InvocationAction: ActionMeta;

    async fn invoke(
        &self,
        grant: Granted<Self::InvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<serde_json::Value, ToolPlaneError>;
}
```

不再使用 `ToolPayloadMatch` / `match_payload` 这种 payload-claim 机制。一个 path
命中后就由对应 tool 自己 parse 和内部流转；如果 `read` 同时支持 file/query/glob，
那它就是一个 aggregate `ReadTool`，内部解析并分流，而不是靠 plane 先看 payload 决定
是否 fallback。

`invoke` 消费 `Granted<Self::InvocationAction>`，所以 typed tool 不能绕过 policy grant
执行。它返回的是 success payload，不是 legacy envelope；旧 `ToolCoreOutcome` 兼容只发生
在 legacy bridge。`ErasedTool` 保持 private/sealed，避免 concrete tool implementer 绕过
plane 的 grant wrapper。这个方法是 granted primitive；普通调用点应使用
`ctx.invoke_tool(path, payload)`，让 app context 负责 resolve/build action、kernel grant 和
audit。

自动 grant 的 shortcut 不放在 `ToolPlane` trait 上。它挂在 app-defined context 上，例如
`ctx.invoke_tool(path, payload)`，因为它是 app runtime orchestration：resolve path、读取
descriptor、构造 invocation action、调用 kernel grant、再把 grant 交给 plane。
`ToolPlane` trait 只表达“已授权 invocation 如何 dispatch”，不知道 kernel、token、pack、
audit sink 或 event id。这样 concrete tool 可以通过 ctx 做受治理的 tool->tool 调用，但
仍然拿不到裸 audit API。

新增 concrete tool 的目标改动面：

1. 增加一个 concrete type 并 `impl ToolImpl<C>`。
2. 在 app/bootstrap/builtin 注册点添加一条 `register(path, Tool)`。

不要为了新增工具去改 dispatcher match、catalog 拼装分支或 policy preflight 分支。

内部 registry 使用 slot storage + path index：

```rust
slotmap::new_key_type! {
    struct ToolSlot;
}

struct ToolPlaneRegistry<C> {
    entries: slotmap::SlotMap<ToolSlot, ToolEntry<C>>,
    paths: BTreeMap<ToolPath, ToolSlot>,
}

struct ToolEntry<C> {
    tool: RegisteredTool<C>,
    registration: ToolRegistration,
}

struct ToolRegistration {
    provenance: ToolProvenance,
}
```

`ToolSlot` 不跨过 `loong-app::tools::plane` 模块边界，不出现在 audit event、
`ToolInvocationAction`、contracts/core 或 concrete tool API 里。不要先做 alias：
一个 path 对应一个 slot；如果后续需要多个 path 指向同一个 tool，必须单独设计 alias
语义，不能把它当成兼容 shim 偷偷塞进 registry。

## Kernel

Kernel 不再提供 typed `Kernel::invoke_tool`，也不持有 typed `tool_plane` 字段。

目标 API：

```rust
Kernel::grant(
    pack_id,
    token,
    impl ActionMeta,
    &ctx,
) -> ActionGrant<A>
```

tool invocation 的治理入口不需要知道 concrete path 类型；它只需要 `ActionMeta` 提供
operation/payload/required capabilities。pack boundary、token boundary、policy pipeline
仍在 kernel 检查。

`Kernel::grant` 不做 tool dispatch，也不拥有 tool registry。grant 过程天然记录
authorization audit：action metadata、required caps、policy report、allow/deny 和
grant id 都在这里落审计。tool invocation 的 denied evidence 因而属于 generic action
grant audit，不需要单独的 `ToolInvocationOutcome::Denied` 或 receipt workaround。

grant 后的 execution outcome audit 属于 grant consumption 边界。对 tool 来说，这个边界
是 `ctx.invoke_tool(...) -> ToolPlane::invoke(...)`；对 fs 来说，是
`Granted<FsReadAction>::run(ctx)`。它只能记录 grant 已经发放之后的 completed / failed /
input error 等结果，不能重复表达 authorization deny。

旧 `Kernel::execute_tool_core` 暂时只服务 legacy fallback。旧 `CoreToolAdapter` /
`ToolExtensionAdapter` 只能被 `LegacyToolPlane` 包住，迁移完后删除。

## Audit

audit 分两层，不再把所有内容塞进 kernel/contract enum。

- kernel 只拥有 `AuditSink`、event id/clock/grant id，以及 generic action authorization
  audit。`Kernel::grant` 对任意 action 统一记录 action metadata、required caps、
  policy report、allow/deny 和 grant id。
- app 拥有 tool-specific execution audit schema。tool path display、tool execution
  outcome、legacy fallback 对比都属于 app runtime 语义；app 可以把这些事件写入 kernel
  提供的 sink，但 kernel 不需要定义这些业务 enum。

typed tool invocation 应该有 audit evidence。tool 调用是 agent/user 可见的治理边界；
authorization allow/deny 由 generic action grant audit 记录；grant 后的执行成功/失败由
app-owned execution audit 记录。否则 typed tool 从 legacy `PlaneInvoked` 迁走后，证据链
反而变少。

tool-specific audit event 不能固化到 contracts/kernel 的 ToolPlane registry key 类型。
此前把 `ToolInvocation { path: ToolPath, ... }` 加到 `AuditEventKind` 里是过度固化。app
层可以为 audit payload 存 `path_display`，因为 audit 需要的是可读、稳定、可关联的 path
表示，不是具体 plane 的 key。`ToolPath` 不应该成为 contracts/core 的全局类型。

app-owned tool execution event 只记录 grant 后结果：

```rust
ToolInvocation {
    pack_id,
    path_display,
    grant_id,
    execution_outcome,
}
```

这里的 `path_display` 是 app audit payload，不是 registry key 类型。不同 `ToolPlane` 可以
有不同 path model，只要 app 在 audit 中给出稳定、可读、可关联的表示。

`ToolInvocationOutcome` 如果保留，应放在 app 层，只能描述 grant 后 execution outcome，
例如 completed / failed / input_error；不能包含 denied 分支，不能隐含 fallback 机制为
kernel contract。

legacy adapter 仍暂时记录旧 `PlaneInvoked`，直到对应工具迁移完成。

### Tool audit failure matrix

authorization audit 由 `Kernel::grant` 强制记录，concrete `ToolImpl` 不拿 audit API。
execution audit 由 `ctx.invoke_tool` / granted action run 边界强制记录。

- invocation grant 被 pack/token/caps 拒绝：`Kernel::grant` 记录 generic action grant deny，
  plane 不执行。
- invocation policy 被 `PolicyPipeline` 拒绝：`Kernel::grant` 记录带 `PolicyReport` 的
  generic action grant deny，plane 不执行。
- payload parse / typed input error：grant 已消费进入 plane，app orchestration 记录
  grant 后 execution failed/input_error，不 fallback。
- concrete tool execution error：app orchestration 记录
  grant 后 execution failed。
- tool 内部 access/action policy denial：domain access 返回 authorization error；app
  orchestration 把本次 tool invocation 记为 failed。domain action 的 policy evidence
  保留在 `PolicyGrantError::Denied { report, ... }`，不要把它伪装成 tool route。
- legacy fallback：继续记录 `PlaneInvoked`，直到该 tool 迁入 typed plane；typed 测试不再
  接受 `PlaneInvoked | ToolInvocation` 这种宽松断言。

当前 `Kernel::grant_tool_invocation` 内部记录部分 deny audit 是迁移期 helper 行为。目标是
generic grant 负责所有 action authorization audit；tool invocation execution audit 留在
`ctx.invoke_tool` 的 grant consumption 边界。

## 当前实现偏差

以下是已经出现、但不应该继续放大的过渡形状：

- `crates/contracts/src/tool_types.rs` 仍定义全局 `ToolPath`，且 `ToolSpec` 仍携带
  `path`。目标是 descriptor 无 path，注册点/plane 才绑定 path。
- `crates/loong-core/src/tool.rs` 里的 `ToolInvocationAction` 持有全局 `ToolPath`。
  这属于 app/plane-owned action；本轮目标是删除 core action，不新增 core generic helper。
- `crates/app/src/tools/plane.rs` 当前 `ToolPlane` 直接使用 contracts `ToolPath`，
  `invoke` 也消费 core `ToolInvocationAction`，内部存储还是
  `BTreeMap<ToolPath, RegisteredTool<C>>`。目标是 app plane 自己定义 path/action，
  并使用 private slot registry + path index；contracts/core 不知道 plane registry key
  或 slot。
- `crates/kernel/src/kernel.rs` 的 `grant_tool_invocation` 当前接收 core
  `ToolInvocationAction` 并在 deny 时记录 audit。目标是拆成 generic action grant +
  app orchestration 负责 tool invocation audit。
- `crates/tools/src/file.rs` 的 `ReadFileTool::spec()` 仍返回带 path 的 `ToolSpec`。
  这是 tool descriptor 与 registration path 未拆开的直接症状。
- `crates/tools/src/file.rs` 的 `ReadFileTool::Output` 仍是 `loong_contracts::ToolOutcome`，且
  `build_outcome` 还在 concrete tool 内构造 `status/payload` envelope。目标是
  `ReadFileTool::Output = ReadFileOutput`，只表达成功 payload；失败用 error path 表达。
- `crates/app/src/tools/mod.rs` 里 typed dispatch、grant、invoke、audit 逻辑还堆在
  `execute_kernel_tool_request`。目标是 app orchestration 拥有这段边界，但函数应更聚焦。
- `crates/app/src/tools/routing.rs` 的 `route_direct_read_tool_request_for_legacy` 名字不准。
  它现在同时承担 read surface normalization 和 legacy bridge；aggregate `ReadTool`
  落地后这块应该删除或拆清楚。
- `AppExecutionContext::capabilities()` 目前基本返回 token allowed caps；还没有 child
  tool context 的 effective caps narrowing。

## `file.read` 当前迁移状态

- 目标是 `read` 作为 aggregate typed tool 进入 app plane，但它内部分出来的 action
  不能聚合。
- `ReadFileTool` 只解析 payload、调用 `ctx.access().fs().read_file(...)`、格式化响应。
- 文件读取副作用发生在 `loong_access::fs`。
- `read { path }` 分支走 `FsResolvePathAction -> GrantedPath -> FsReadAction`。
- `read { query }` / `read { pattern }` / `read { glob }` 后续迁入 typed `ReadTool`
  内部分流，但分别落到 content-search / glob-path action。迁移前只能作为明确 TODO 的
  legacy bridge，不作为新 plane 的 payload claim 设计。
- `read { path, offset: 0 }` 是 typed input error，不 fallback。

## 下一步

按最小提交顺序推进：

1. 删除 `loong_contracts::ToolOutcome`，把 typed tool output 改成 `Result<Value, E>`：
   - 先写/调整失败测试：
     - `loong-core` 的 erased invoke 返回 success payload，不返回
       `loong_contracts::ToolOutcome { status: "ok", payload }`；
     - `loong-tools` 的 read tool success output 是专门类型，不再直接等于
       `loong_contracts::ToolOutcome`；
     - app 的 `file.read` / `read { path }` 外部响应保持不变；
   - 修改 `crates/loong-core/src/tool.rs`：
     - 将 `ToolImpl::Output` 约束从 `Into<loong_contracts::ToolOutcome>` 收窄到 typed
       success payload，首选 `Into<serde_json::Value>`，除非实现时发现需要一个极小的
       本地 trait；
     - `RegisteredTool::invoke` 只做 sealed type erasure 和 concrete tool 调用，返回
       `Result<serde_json::Value, ToolExecutionError>`；不要在 core/erased 层拼 `"ok"`
       status；
     - 注释说明 legacy envelope 是 `ToolCoreOutcome`，不是 erased tool contract；
       concrete tool 的成功和失败必须由 `Result<Output, ToolExecutionError>` 表达；
   - 修改 `crates/tools/src/file.rs`：
     - 增加 `ReadFileOutput` 专门 struct，字段承载 read 响应所需 payload 数据；
     - `impl From<ReadFileOutput> for serde_json::Value`，把旧 payload JSON 构造搬到这里；
     - `ReadFileTool::Output = ReadFileOutput`；
     - `build_outcome` 改名为 `build_output`，返回 `Result<ReadFileOutput, String>` 或
       直接返回 `Result<ReadFileOutput, ToolExecutionError>`；
     - 不在 concrete tool 内构造 `"ok"` status；
   - `crates/contracts/src/tool_types.rs` 删除 `loong_contracts::ToolOutcome`；不要为了兼容
     concrete tool 或 erased tool 再给它加 helper；
   - `crates/app/src/tools/routing.rs` 继续只在 legacy bridge 边界把 typed success payload
     包成 `ToolCoreOutcome`，不要把 legacy envelope 泄漏进 `loong-core` 或
     `loong-tools`；
   - 验证：`cargo test -p loong-core tool`、
     `cargo test -p loong-tools --no-default-features --features file`、
     `cargo check -p loong-app`、
     `cargo test -p loong-app kernel_routed_file_read -- --nocapture`、
     `cargo test -p loong-app file_read -- --nocapture`、
     `cargo fmt --all -- --check`、`git diff --check`。

2. 把 `ToolInvocationAction` 收回 `loong-app::tools::plane`：
   - 先写失败测试：`loong-core` 不再需要 `ToolPath` 才能编译 tool abstraction，
     app typed tool invocation 仍然产生 app-owned tool execution audit；
   - 在 `crates/app/src/tools/plane.rs` 附近定义 plane-local `ToolPath` 和
     `ToolInvocationAction`；
   - `AppToolPlane` 使用自己的 `Path`，不再直接使用 contracts `ToolPath`；`ToolPlane`
     trait 不要求公开 path 类型；
   - action payload 继续携带 agent/tool 原始 `payload: Value`，grant 后 plane 再 parse
     concrete input；
   - `ToolInvocationAction` 持有 app plane 自己的 path display/registry path，不把
     concrete path type 泄漏进 contracts/core；
   - kernel grant API 只接受 concrete `ActionMeta` 和 context，返回 `ActionGrant<A>`；
     kernel 不知道 app plane path type，也不返回 `AuthorizedToolInvocation` receipt；
   - 删除 `loong-core::tool::ToolInvocationAction`；本轮不添加 core generic helper；
   - 更新注释：core 只承载 tool abstraction，`loong-app::tools::plane` owns typed
     plane，kernel 只 grant/audit action，不执行 typed tool；
   - 完成线：
     - `crates/loong-core/src/tool.rs` 不再 import `loong_contracts::ToolPath`；
     - `crates/app/src/tools/plane.rs` 不再 import contracts `ToolPath` 或 core
       `ToolInvocationAction`；
     - `Kernel::grant_tool_invocation` 被 generic action grant 取代，或至少不再接收
       path-specific action type；
     - app typed read path 通过 `ctx.invoke_tool` 进入，内部先 grant invocation action，
       再调用 `ToolPlane::invoke`；
     - pack/token/caps/policy denial 由 generic action grant audit 记录，不进入 app tool
       execution outcome；
     - typed path tests 断言 generic action grant audit + grant 后 app-owned tool execution
       audit，不接受 legacy `PlaneInvoked` 兜底。
   - 验证：`cargo test -p loong-core tool`、`cargo test -p loong-kernel tool_invocation`、
     `cargo test -p loong-app kernel_routed_file_read`、`cargo check -p loong-core -p
     loong-kernel -p loong-app -p loong-tools -p loong`。

3. 将 `ToolPlane` 内部存储改成 slot registry + path index：
   - 根 `Cargo.toml` 增加 `slotmap = "1"` workspace dependency，`crates/app/Cargo.toml`
     使用 `slotmap.workspace = true`；
   - 在 `crates/app/src/tools/plane.rs` 定义 private `ToolSlot`，不要 re-export；
   - 将 `AppToolPlane<C>` 从 `BTreeMap<ToolPath, RegisteredTool<C>>` 改为
     `entries: slotmap::SlotMap<ToolSlot, ToolEntry<C>>` +
     `paths: BTreeMap<ToolPath, ToolSlot>`；
   - `ToolEntry<C>` 只保存 `RegisteredTool<C>` 和 `ToolRegistration`；
     `ToolRegistration` 先至少承载 provenance，后续 registration time/source 也放这里；
     注释说明 slot 是内部注册句柄，不是 public identity；entry 不保存 path，避免和
     `paths` index 重复；
   - `register(path, tool)` 先检查 `paths` duplicate，再 insert entry，最后写入
     `paths.insert(path, slot)`；不要允许 alias；
   - `invoke(grant, ctx)` 先从 action 取 path，经 `paths` 查 slot，再从 `entries`
     取 entry 并调用 tool；缺失 slot 返回 `ToolPlaneError::ToolNotFound(path_display)`；
   - 测试覆盖：duplicate path 不产生第二个 entry、missing path 返回 not found、
     invoke 仍消费 grant 并执行目标 tool、slot 不出现在 public audit payload；
   - 验证：`cargo test -p loong-app tools::plane`、`cargo check -p loong-app`、
     `cargo fmt --all -- --check`、`git diff --check`。

4. 实现 effective caps / child context narrowing：
   - `AppExecutionContext` 增加 explicit effective caps 字段，`PolicyContext::capabilities()`
     返回该字段，而不是临时从 token 拷贝；
   - 顶层 tool invocation context 由 token caps 初始化；
   - tool 调 tool 时，根据 child tool descriptor default caps 和 optional override 构造
     child effective caps；
   - override 必须是 default caps 的子集，否则 typed input error / policy deny，不能静默
     提升；
   - child context 继承 kernel/workspace/config 等 ref 字段，但 caps 字段使用缩窄后的
     集合；
   - 测试覆盖：override 缩窄生效、override 扩大被拒、父 context 缺 cap 时 child 不会获得
     该 cap、domain action gate 读取的是 child effective caps。

5. 清理 tool descriptor/path 耦合：
   - `ToolImpl::spec()` 返回无 path descriptor；
   - `RegisteredTool` 只保存 descriptor/provenance/registration metadata；
   - `ToolPlane::register(path, tool)` 组合 path + descriptor；
   - `ReadFileTool::spec()` 不再硬编码 `"read"`。

6. 收敛 app typed dispatch 边界：
   - 从 `execute_kernel_tool_request` 中抽出一个聚焦的 app orchestration 边界；
   - 该边界最终落到 `ctx.invoke_tool(path, payload)`；内部只做 app plane resolve/build
     action -> kernel grant（自动 authorization audit）-> plane `invoke` -> grant
     后 execution audit；
   - 不引入 `AuthorizedToolInvocation` receipt workaround。

7. 改 `read` 为 aggregate typed tool：
   - 删除 payload-claim/fallback 思路；
   - `ReadTool` 内部解析 `path/query/pattern/glob`；
   - `path/query/glob` 分别构造不同 action；
   - `read { path, offset: 0 }` 是 typed input error，不 fallback；
   - `read { query }` / `read { pattern }` / `read { glob }` 迁入 typed path 后，旧
     direct read legacy bridge 删除。

8. 继续迁移剩余 legacy side-effect tools：
   - write/edit/config.import 按同样 access-backed action 模式迁移；
   - 迁移完成后删除 `FilePolicyExtension` 对应旧分支；
   - 逐步清空 `Kernel::execute_tool_core` 调用面，再删除 `LegacyToolPlane` 和 adapter
     trait。

9. 测试清理：
   - typed tool 测试只接受 `ToolInvocation` audit；
   - legacy adapter 测试只接受 `PlaneInvoked` audit；
   - 不用 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 每个最小提交跑对应 targeted tests、`cargo check` 和 `git diff --check`。

10. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration。
