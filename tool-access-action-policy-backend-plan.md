# Tool Access / Action / Policy / Backend 临时计划

这是临时设计计划，不并入 `docs/`。本文按后续讨论后的形状更新：

- concrete unified context 由 app 定义；
- kernel 通过 context type factory 泛型连接 policy/access/tool；
- policy 绑定 context factory，不通过 `PolicyEngine` 间接拿 `Cx`；
- `PolicyEngine` 内不再定义或持有 factory / `Cx` associated type；
- migrated side effect 只能从 access 进入；
- 当前 object-safe action metadata trait 改名为 `ActionMeta`；
- 新的 executable action trait 是 `Action<Cx>: ActionMeta + Sized`，其 `run`
  直接接受 `Granted<Self>`；
- `Granted<A>` 提供 port，消费授权 token 后调用 `A::run(...)`；
- backend 不是必需抽象；如果后续存在，也只是 action run 内部实现细节，不能暴露给
  tool。

## 已确认原则

- 敢于破坏性改动，不为已迁移路径保留 alias、proxy、compatibility shim 或
  root re-export 来拖延收口。
- 各部件尽力减少耦合：contracts 放稳定数据，core 放行为 trait，kernel 放治理
  流程，access 放 domain side-effect boundary，app 放 concrete context/config/policy
  wiring。
- migrated tool 的副作用 only access can do。tool/helper/adapter/kernel policy 都不
  直接执行 migrated side effect。
- tool helper 只做 payload parsing、runtime narrowing、调用 access、格式化响应。
  它不拼授权链，不读全局 config 来决定 policy，不直接接触 backend。
- concrete unified context 由 app 定义。kernel 不固定 app context 字段，只通过
  `ContextFactory` type factory 连接 policy/access/tool。泛型传染是有意的：它强制
  调用点和测试通过 trait 约束获取 context 能力，而不是偷用 concrete context 字段。
- `ContextFactory` 在 kernel/policy/access/tool 之间作为显式泛型参数 `C` 传递，
  不作为 `Kernel` 的 associated type 再间接取用。
- `ContextFactory` 是 type-level factory，只有 GAT，没有 `create` / `build`
  method。context 构造发生在 app/runtime 边界，不发生在 core trait 里。
- `Policy` / `PolicyAny` 绑定 `ContextFactory`；`PolicyEngine` 不拥有 factory，
  也不定义 `type Cx` / `type Context`。
- `Policy` 和 `PolicyAny` API 应保持一致，例如 `name() -> Cow<'static, str>`。
- 业务 policy 不依赖 app concrete context；它应为任意满足所需 view trait 的
  `C::Cx<'_>` 实现。后续代码注释要明确这点，避免 policy 偷偷绑定
  `AppExecutionContext`。
- 需要额外信息时，用小 view trait 表达使用点需求，例如 `FsAccessContext`、
  `ExecutionView`、`ToolInvocationView`。不要定义一个大 deps struct，也不要为每个
  domain 在 kernel 上加 getter。
- `required_capabilities` 是 `ActionMeta` 的属性；运行时 root、tool config、
  workspace view 不是 action 字段。
- `Action<Cx>` 是 executable action：`run(granted: Granted<Self>, cx: &Cx)` 是唯一
  side-effect implementation hook。raw action 不直接执行；正常调用经由
  `Granted<A>::run(cx)` port。
- `CanonicalPath` 负责 path normalization、allowed roots、`..` 和 symlink escape。
  `FsReadAction` 不持有 workspace root / file root。
- `PolicyPipeline` 是 ordered pipeline。`Allow` / `Deny` 终止整个 pipeline，
  `Continue` 继续当前 subchain，`Advance` 跳到下个 subchain；无 terminal decision
  时 default deny。这里主要靠注释讲清控制流语义，不额外引入 wrapper 层。
- config-driven policy 的入口在 app 组装处：app 读取并规范化 config，把需要的配置
  显式构造成 policy，再注册进 pipeline。access 不读取 app config，tool helper 不在
  access 前执行 config-backed policy 分支。
- deny 路径保持结构化。`PolicyReport` 应沿 error path 传到 response 边界，避免
  字符串匹配或 `is_policy_denial()` 这类猜测 helper 成为长期方案。
- 不要求抽统一 backend trait。执行边界先表达为
  `Action<Cx>::run(Granted<Self>, &Cx)`；若后续 action 内部需要 backend，它只是 run
  的实现细节。
- 测试跟随对应模块放置，例如 fs access 测试放在 `fs/tests.rs` 这类局部位置；
  不新增无归属的大型跨模块测试文件。
- 注释只服务边界理解：要标出 config -> policy、kernel registration、legacy
  fallback 的所有权位置；还要说明 policy 不依赖 app concrete context、完整
  `ToolRuntimeConfig` 不能成为跨层 ABI、pipeline terminal/advance 语义、`ActionMeta`
  和 `Action<Cx>` 的分工、`Granted<A>::run(cx)` 是授权到执行的 port。不写空泛叙述。
  后续实现时同步代码 comment 和 docs；本计划先记录这两个同步点。

## 核心模型

工具侧只表达意图，副作用必须经由 access：

```text
tool adapter
  -> ToolCoreContext::access()
  -> AccessCx<'_, C>
  -> AccessCx::fs()
  -> FsAccess<'_, C, P>::read_file(path)
      -> CanonicalPath::resolve(path, ctx fs view)
      -> FsReadAction::new(CanonicalPath)
      -> PolicyPipeline<C>::grant(ctx, FsReadAction)
      -> ActionGrant<FsReadAction>
      -> Granted<FsReadAction>::run(ctx)
      -> side effect
```

`Access`、`ActionMeta`、`Action<Cx>`、`Policy` 是治理语义。tool/app 不直接拼授权链，不直接
读取 policy context 字段，也不直接接触 backend handle。

## 分层边界

### `loong-contracts`

只放稳定数据 contract，不放 domain view，不放 app context，不放执行逻辑。

适合放在 contracts 的内容：

- `Capability` / `CapabilityToken`
- `ExecutionPlane` / `PlaneTier`
- `PolicyDecision`
- `PolicyGrant`
- `PolicyEntry`
- `PolicyEvaluation`
- `PolicyOutcome`
- `PolicyReport`
- `GrantId` / `PolicyId`
- tool/runtime/memory request/outcome 数据结构

不放：

- `ContextFactory`
- `PolicyContext`
- `ActionMeta`
- `Action<Cx>`
- `Policy`
- `PolicyAny`
- `PolicyEngine`
- `FsAccessContext`
- workspace root / file root / tool config 视图

这些是行为 trait 或 domain/app view，属于 `loong-core`、`loong-access` 或 app。

### `loong-core`

放跨 kernel/access/policy 共用的行为 contract：

```rust
/// Type-level factory for the execution context used by policy/access/tool code.
///
/// This trait maps a borrow lifetime to the concrete context type. It does not
/// construct context values; app/runtime code owns value construction.
pub trait ContextFactory {
    type Cx<'a>: PolicyContext
    where
        Self: 'a;
}
```

这个 factory 是 type-level factory，只有 GAT，没有 `create` / `build` method。
具体 context 的构造由 app/runtime 边界负责，kernel 不通过 factory method
构造 app context。

当前 object-safe action metadata 改名为 `ActionMeta`，供 policy/audit/any-policy
观察 action，不携带 execution output/error：

```rust
pub trait ActionMeta {
    fn kind(&self) -> &'static str;

    fn operation(&self) -> Cow<'static, str>;

    fn audit_resource(&self) -> Option<Cow<'static, str>>;

    fn required_capabilities(&self) -> BTreeSet<Capability>;
}
```

Executable action 绑定到具体 context，并且 `run` 直接接受 `Granted<Self>`：

```rust
pub trait Action<Cx>: ActionMeta + Sized {
    type Output;
    type Error;

    fn run(granted: Granted<Self>, cx: &Cx) -> Result<Self::Output, Self::Error>;
}
```

`Granted<A>` 提供唯一推荐 port：

```rust
impl<A> Granted<A> {
    pub fn run<Cx>(self, cx: &Cx) -> Result<A::Output, A::Error>
    where
        A: Action<Cx>,
    {
        A::run(self, cx)
    }
}
```

代码注释和 docs 需要同步说明这两个点：

- `ActionMeta` 是 object-safe metadata view，服务 policy/audit/type-erased
  policy；
- `Action<Cx>::run(Granted<Self>, &Cx)` 是 action 自己的执行实现，正常入口是
  `Granted<A>::run(&Cx)`。

Policy 直接绑定到 context factory：

```rust
#[async_trait]
pub trait Policy<C: ContextFactory, A: ActionMeta>: Send + Sync {
    fn name(&self) -> Cow<'static, str>;

    async fn grant(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyGrant;
}

#[async_trait]
pub trait PolicyAny<C: ContextFactory>: Send + Sync {
    fn name(&self) -> Cow<'static, str>;

    async fn grant(&self, ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyGrant;
}
```

业务 policy 的 impl 应继续保持 context-generic，只约束它实际需要的 view：

```rust
#[async_trait]
impl<C> Policy<C, FsReadAction> for FsReadFilenameDenyPolicy
where
    C: ContextFactory,
    for<'a> C::Cx<'a>: FsReadPolicyView,
{
    fn name(&self) -> Cow<'static, str> {
        "fs_read_filename_deny".into()
    }

    async fn grant(&self, ctx: &C::Cx<'_>, action: &FsReadAction) -> PolicyGrant {
        // uses FsReadPolicyView, not AppExecutionContext fields
    }
}
```

实现处应加简短注释说明：policy 不能依赖 app concrete context；需要的输入通过
policy 构造参数或小 view trait 表达。

`PolicyEngine` 只作为执行器 trait，被 context factory 参数化；它不定义
`type Cx`，也不定义 `type Context`：

```rust
#[async_trait]
pub trait PolicyEngine<C: ContextFactory>: Sync {
    async fn decide<A: ActionMeta + 'static>(
        &self,
        ctx: &C::Cx<'_>,
        action: &A,
    ) -> PolicyReport;

    async fn next_grant_id(&self) -> GrantId;

    async fn grant<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, PolicyGrantError>
    where
        A: 'static,
    {
        // capability gate, decide, then mint ActionGrant
    }
}
```

`PolicyContext` 是最小 root view，主要服务 capability gate。旧 `ActionContext`
已删除；`ExecutionPlane` / `PlaneTier` 仍可作为 invocation/global execution fact
保留在具体 context 里。后续若有 policy/access 需要读取它们，再拆成更准确的小
view trait，例如 `ExecutionView`，只在真正需要的位置约束。

### `loong-kernel`

kernel 固定治理流程，不固定 app context 字段。

目标形状：

```rust
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    // packs, tokens, planes, adapters, audit, clock ...
}

pub struct PolicyPipeline<C: ContextFactory> {
    pre_policies: Vec<RegisteredAnyPolicy<C>>,
    typed_policies: AnyMap<... RegisteredPolicy<C, A> ...>,
    fallback_policies: Vec<RegisteredAnyPolicy<C>>,
    // legacy extensions only while unmigrated tools remain
}

impl<C: ContextFactory> PolicyEngine<C> for PolicyPipeline<C> {
    // pre -> action -> fallback
}
```

`PolicyPipeline` 的语义固定：

- `pre` stage：broad gates，先于 typed action policy；
- `action` stage：按 concrete action type 注册；
- `fallback` stage：兼容旧 action 的 broad policy；
- `Allow` / `Deny` 终止整个 pipeline；
- `Continue` 继续当前 subchain；
- `Advance` 跳到下一个 subchain；
- 没有 terminal decision 时 default deny；
- `PolicyReport` 记录完整 evaluations 和 outcome。

`Kernel<C>` 是把 policy engine、tool context、access facade 统一到同一个
`C::Cx<'a>` 的地方。它不应该有 `fs_policy_context()` / `browser_policy_context()`
这类 domain getter。

当前代码状态：`Kernel<C>`、`PolicyPipeline<C>`、`ToolPlane<C>`、
`ToolCoreContext<'a, C>`、`AccessCx<'a, C>` 已落地。`KernelContextFactory`
只是默认/legacy kernel context factory；生产 app context 仍待 app 定义。

### `loong-access`

access crate 定义 side-effect domain 的治理入口和 action。

以 fs 为例：

- `FsAccess`
- `FsReadAction`
- `FsAction`
- `CanonicalPath`
- `FsReadOutput`
- `FsAccessContext`

`FsAccessContext` 是 fs domain view trait，放在 `loong-access::fs`：

```rust
pub trait FsAccessContext {
    fn fs_resolution_root(&self) -> &Path;

    fn fs_allowed_roots(&self) -> &[PathBuf];
}
```

`CanonicalPath` 负责 path normalization、relative resolution、allowed roots、
`..` escape、symlink escape。`FsReadAction` 不携带 workspace root / file root；
root 是 context/policy 的需求，不是 action 自身字段。

`FsAccess::read_file` 的目标顺序固定：

```text
resolve path
  -> build FsReadAction
  -> policy grant
  -> Granted<FsReadAction>::run(ctx)
  -> read file
```

当前实现里 file read side effect 发生在 `loong_access::fs` 内，这是已接受的
vertical slice。后续如果引入 backend，也必须由 access/kernel 内部调用，并消费
`Granted<ConcreteAction>`；backend 可以是 concrete method 或函数，不必抽统一
trait。tool 仍然不能接触 backend。

### `loong-app`

app 定义 concrete unified context 和业务 policy。

目标形状：

```rust
pub struct AppContextFactory;

impl ContextFactory for AppContextFactory {
    type Cx<'a> = AppExecutionContext<'a>;
}

pub struct AppExecutionContext<'a> {
    // kernel-known invocation facts
    pub pack: &'a VerticalPackManifest,
    pub token: &'a CapabilityToken,
    pub now_epoch_s: u64,
    pub plane: ExecutionPlane,
    pub tier: PlaneTier,
    pub request_parameters: Option<&'a serde_json::Value>,

    // app-known execution facts
    pub workspace_root: &'a Path,
    pub file_root: Option<&'a Path>,
    pub fs_allowed_roots: &'a [PathBuf],
    pub tool_name: Option<&'a str>,
}
```

字段只是示意；关键是 concrete context 由 app 拥有。它按需实现小 view trait：

```rust
impl PolicyContext for AppExecutionContext<'_> { ... }
impl FsAccessContext for AppExecutionContext<'_> { ... }
impl ExecutionView for AppExecutionContext<'_> { ... }
impl ToolInvocationView for AppExecutionContext<'_> { ... }
```

tool adapter 只做：

- payload parsing；
- runtime narrowing / root view 选择；
- 调用 `ctx.access().fs().read_file(&path)`；
- 格式化 legacy response。

tool helper 不执行 migrated read side effect。

#### Config -> Policy 路径

配置属于 app 层输入；policy 可以依赖配置，但这个依赖必须由 app 在组装时显式表达。
access 和 tool helper 都不应该直接读取全局配置来决定 policy。

当前 `deny_read_filenames` 路径是：

```text
loong.toml / LoongConfig
  -> ToolConfig.fs.deny_read_filenames
  -> ToolRuntimeConfig::from_loong_config(...)
  -> ToolRuntimeConfig.fs.deny_read_filenames
  -> app::context::policy_pipeline_for_tool_runtime_config(...)
  -> PolicyPipeline::push_fs_read_filename_deny_policy(...)
  -> FsReadFilenameDenyPolicy
  -> typed Policy<FsReadAction>
```

这个方向是对的：app 把 config 投影成 runtime config，再用 runtime config 构造并
注册 policy。tool 只拿到已经组装好的 kernel/pipeline/context，不自己根据 config
决定授权。

后续 config-driven policy 都应遵循同一路径：

```text
app config
  -> normalized runtime config
  -> app-owned policy pipeline construction
  -> typed/broad policy registration
  -> access/action grant time evaluation
```

不要把 config 读取放进 `loong-access`；不要让 access 方法接收 config；不要让
tool helper 在调用 access 前执行 config-backed policy 分支。需要配置的 policy
应该把配置作为自己的显式构造参数、字段或小 view trait 依赖；policy impl 仍然要
对任意满足这些 trait 的 context 生效，而不是绑定 app concrete context。

## 当前执行状态

### 已完成

- `loong-access` 已创建，依赖 `loong-core` 和 `loong-contracts`。
- `loong-access` 当前只被 `loong-kernel` 依赖，app 不直接依赖 access。
- `FsReadAction` / `FsAction` / `CanonicalPath` / `FsAccess::read_file` 已落地。
- `file.read` / `read` path mode 已迁移到：
  `ToolCoreContext::access().fs().read_file(...)`。
- migrated read 已退出 `direct_policy_preflight` 的 file 分支。
- `FilePolicyExtension` 不再覆盖 read，暂时只服务未迁移的 file surfaces。
- `PolicyPipeline` 已有 typed registry、pre/action/fallback stages、`PolicyReport`。
- `ContextFactory` 已在 `loong-core` 落地，只有 GAT，没有 create/build method。
- `Policy` / `PolicyAny` / `PolicyEngine` 已改为显式 `C: ContextFactory` 泛型。
- `PolicyEngine` 不再定义 `type Cx`。
- `Kernel<C>` / `PolicyPipeline<C>` / `ToolPlane<C>` 已泛型化。
- `ToolCoreContext<'a, C>` 携带 `C::Cx<'a>`。
- kernel facade `AccessCx<'a, C>` 只保留 context factory 泛型，不再暴露额外
  `K: Kernel` 参数。
- `Kernel<C>` 构造函数已泛型化，可以实例化非默认 context factory。
- `ActionContext` / `WorkspacePolicyContext` 已从 `loong-core` 删除。
- `KernelPolicyContext` 已降为默认/legacy context：它保留 pack/token/time/request
  params、plane/tier invocation facts 和当前临时 fs view，不再通过 core root trait
  传播。
- `PolicyDecision` 已是 `Allow` / `Deny` / `Continue` / `Advance`。
- `deny_read_filenames` 已作为 typed `FsReadAction` policy 接入。
- `deny_read_filenames` 已有 config -> runtime config -> policy pipeline ->
  typed policy registration 路径。

### 仍是过渡形状

- 默认 kernel 入口仍使用 `KernelPolicyContext<'a>`。
- app 仍使用默认 `KernelContextFactory`，尚未拥有 concrete unified context。
- `ToolCoreContext::with_fs_root_view(...)` 仍在临时拼 fs view。
- `loong_access::fs::FsAccess` 内部只持有 policy engine 引用，不再持有 kernel host。
  该类型仍有 `P: PolicyEngine<C>` 泛型，因为 `PolicyEngine<C>` 需要 generic
  action grant，不能直接做成普通 trait object。
- fs read execution boundary 仍未收敛到 `Action<Cx>::run` / `Granted<A>::run(cx)`。
- `fs_read_error_is_policy_denial` 仍是临时 deny 分类 helper。

### 尚未完成

- app-defined `AppExecutionContext<'a>` 尚未落地。
- `ActionMeta` / `Action<Cx>` split 尚未落地。
- `Granted<A>::run(cx)` port 尚未落地。
- HTTP / shell / browser / memory 等 tool family 尚未迁移。
- child action constructor pattern 尚未落地。
- `PolicyReport` 尚未贯穿 access/tool error 到 Agent-facing response。

## 实现计划

### 1. 收敛 context type factory

在 `loong-core` 增加只有 GAT 的 `ContextFactory`：

```rust
pub trait ContextFactory {
    type Cx<'a>: PolicyContext
    where
        Self: 'a;
}
```

同时重新评估并清理：

- `ActionContext` 作为 root bound 的用法；
- `WorkspacePolicyContext`；
- 当前 `KernelPolicyContext` 字段和职责。

不要给 `ContextFactory` 加 `create` / `build` method。app 自己构造 concrete
context，factory 只提供类型族。

除 `ContextFactory` 自己的 GAT 外，`C` 一律作为泛型参数显式传递，不通过
`Kernel::C` / `Kernel::ContextFactory` 这类 associated type 镜像：

```rust
PolicyEngine<C>
Policy<C, A>
PolicyAny<C>
Kernel<C>
AccessCx<'a, C>
ToolCoreContext<'a, C>
```

`loong_access::fs::FsAccess` 内部保留 `P: PolicyEngine<C>` 泛型；access crate 不依赖
concrete `loong_kernel::Kernel<C>`，也不持有整个 kernel host。

### 2. 拆 ActionMeta / Action<Cx>

先把当前 object-safe `Action` metadata trait 改名为 `ActionMeta`：

```rust
ActionMeta
  - kind
  - operation
  - audit_resource
  - required_capabilities
```

再引入 executable action trait：

```rust
Action<Cx>: ActionMeta + Sized
  - type Output
  - type Error
  - run(Granted<Self>, &Cx)
```

`Granted<A>` 增加 port：

```rust
Granted<A>::run<Cx>(&Cx)
where
    A: Action<Cx>
```

`ActionMeta` 的 docs/comment 要说明它只是 policy/audit/type-erased metadata view。
`Action<Cx>` 的 docs/comment 要说明它是 side-effect implementation hook，并且
`run` 必须消费 `Granted<Self>`。`Granted<A>::run(cx)` 的 docs/comment 要说明它是
授权 token 到执行的推荐入口。

### 3. 改 Policy / PolicyAny / PolicyEngine 泛型

目标：

```rust
Policy<C, A: ActionMeta>
PolicyAny<C>
PolicyEngine<C>
PolicyPipeline<C>
```

禁止：

```rust
Policy<P: PolicyEngine, A>
PolicyEngine { type Cx<'a>; }
PolicyEngine { type Context; }
Kernel { type C; }
```

`PolicyEngine` 不拥有 factory，只被 `C: ContextFactory` 参数化。

### 4. 泛型化 kernel 和 tool context

把 concrete kernel 迁到：

```rust
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    // ...
}
```

`ToolCoreContext` 携带同一个 `C::Cx<'a>`：

```rust
pub struct ToolCoreContext<'a, C: ContextFactory> {
    kernel: &'a Kernel<C>,
    context: C::Cx<'a>,
}
```

`AccessCx` 同样显式携带 `C`，不从 `K` 的 associated type 反查 context factory。
作为 kernel-defined facade，它直接持有 `&Kernel<C>`，所以不需要额外的 `K`
泛型：

```rust
pub struct AccessCx<'a, C: ContextFactory> {
    kernel: &'a Kernel<C>,
    context: C::Cx<'a>,
}
```

`AccessCx` 继续是 kernel ref + unified context。工具调用保持类似：

```rust
ctx.access().fs().read_file(&path).await
```

### 5. App 落地 concrete unified context

在 app 定义：

- `AppContextFactory`
- `AppExecutionContext<'a>`
- 需要的小 view trait impls

把当前 `KernelPolicyContext::with_fs_root_view(...)` 的信息移到 app context 构造处。
fs root view 不应由 action 持有，也不应由 access helper 临时塞入 kernel context。

### 6. 固化 Config -> Policy 组装边界

保留并推广当前 `deny_read_filenames` 的方向：

- config parsing 只产出 config 数据；
- runtime config 做 normalization / narrowing；
- app context/bootstrap 根据 runtime config 构造并注册 policies；
- kernel pipeline 只接收已构造好的 policy；
- access/tool helper 不反向读取 app config 来决定授权。

在引入 `AppContextFactory` 后，config 仍不进入 `ContextFactory` trait 本身。
factory 只提供 context type family；policy pipeline construction 可以读取 normalized
runtime config 来构造 policy。

### 7. 清理 legacy policy/context 形状

删除或替换：

- `KernelPolicyContext`，或至少降为 app/test-only context；
- 当前 object-safe `Action` metadata trait；
- `Policy<PolicyPipeline, A>` 这种 engine-bound policy impl；
- `ToolCoreContext::with_fs_root_view(...)`。

已完成：

- `ActionContext` root bound 已删除；
- `WorkspacePolicyContext` 已删除。

保持破坏性改动优先，不为已迁移路径保留 alias / compatibility shim。

### 8. 优化 deny/report 路径

把 policy denial 作为结构化 authorization error 贯穿：

```text
PolicyReport
  -> PolicyGrantError
  -> FsAccessError
  -> ToolPlaneError / ToolCoreOutcome
  -> Agent-facing response
```

目标是删除 `fs_read_error_is_policy_denial()` 这类猜测 helper。Agent-facing
提示应能说明：

- 哪个 policy 拒绝；
- 拒绝对象是什么；
- 是否应该换路径、请求授权，或停止重试。

测试重点仍放在 `file.read` deny：无读取副作用、错误码稳定、legacy preflight
不参与。

### 9. Action run / execution boundary

当前 fs read side effect 在 `loong_access::fs` 内，这是可接受的 vertical slice。
目标不是先抽 backend，而是把执行入口收敛为 action run：

- action 自己实现 `Action<Cx>::run(Granted<Self>, &Cx)`；
- access 消费 policy grant 后调用 `grant.granted.run(ctx)`；
- `Granted<A>::run(ctx)` 是授权 token 到 side effect 的 port；
- action run 可以忽略 ctx，例如 `FsReadAction` 只需要 canonical path；
- action run 也可以通过 `where Cx: SomeView` 获取执行所需上下文；
- 如果后续某个 action 内部需要 backend，backend 只是 `run` 的实现细节，不要求统一
  trait。

不要把 backend registry 作为运行时工具选择面，也不要把 backend handle/function
暴露给 tool-facing API。

### 10. 迁移剩余 tool families

在 context/policy 泛型收口后，再迁移：

1. write/edit/config.import；
2. shell/bash execution；
3. HTTP/web request；
4. browser session operations；
5. memory/session durable state changes；
6. hidden specialized tools。

每个迁移完成后，tool helper 只能解析 payload、调用 access、格式化响应。

### 11. Child action constructor pattern

凡是 child action 语义上需要 parent authorization，constructor 接收
`Granted<SuperAction>`：

```rust
impl BrowserClickAction {
    pub fn new(
        granted: Granted<BrowserSessionAction>,
        link_id: LinkId,
    ) -> Result<Self, BrowserActionError> {
        let session = granted.into_action();
        // validate link_id under authorized session scope
        Ok(Self {
            session_id: session.session_id,
            link_id,
        })
    }
}
```

如果某个 domain 需要在一个授权 scope 内多次派生操作，应显式设计 lease/session，
不要 clone grant。

## 禁止形态

- tool 直接执行 migrated side effect。
- tool 直接 name、construct 或 invoke backend handle/function / concrete backend。
- tool-facing API 暴露 policy context、domain view trait、backend handle。
- `Policy` 通过 `PolicyEngine` 获取 `Cx`。
- `PolicyEngine` 定义 `type Cx` / `type Context`。
- `ContextFactory` 带 create/build method。
- `ActionMeta` 携带 execution output/error/run。
- `Action<Cx>` 不消费 `Granted<Self>` 就执行副作用。
- policy impl 依赖 `AppExecutionContext` 这类 app concrete context，而不是依赖小
  view trait。
- 为每个 domain 在 kernel 上增加 `fs_policy_context()` 这类 getter。
- action 持有 workspace root / file root 这类 invocation context。
- side-effect implementation 接受 ungranted action。
- tool/app 直接调用 `Action<Cx>::run(...)`，绕过 `Granted<A>::run(cx)` port。
- 为已迁移路径保留 alias/helper compatibility layer。
- access/tool helper 直接读取 app config 来决定 policy。
- 把整个 `ToolRuntimeConfig` 暴露成跨层逃逸口，让 policy/access 依赖 app config
  的 concrete shape。

## 验收标准

- `ContextFactory` 只有 GAT。
- `Policy<C, A>` / `PolicyAny<C>` / `PolicyEngine<C>` / `PolicyPipeline<C>` 使用同一个
  `C: ContextFactory`。
- `PolicyEngine` 内没有 factory 或 `Cx` associated type。
- 当前 object-safe action metadata trait 已改名为 `ActionMeta`。
- executable `Action<Cx>` 通过 `run(Granted<Self>, &Cx)` 表达 side-effect
  implementation。
- `Granted<A>::run(cx)` 是授权 token 到 action run 的唯一推荐 port。
- concrete app context 由 app 定义并实现所需 view traits。
- 业务 policy 为任意满足所需 view trait 的 `C::Cx<'_>` 实现，不绑定
  `AppExecutionContext`。
- kernel 只通过泛型连接统一 context，不固定 app context 字段。
- `file.read` 继续只通过 `ctx.access().fs().read_file(...)` 执行读取。
- migrated side effect 不经过 direct preflight / `FilePolicyExtension`。
- config-driven policy 经由 app config -> normalized runtime config ->
  app-owned policy construction -> policy registration 进入 pipeline。
- `Granted<A>` 仍不可被 `loong-core` 外部伪造。
- 代码 comment/docs 说明 `ActionMeta` / `Action<Cx>` 分工，以及
  `Granted<A>::run(cx)` 的执行边界语义。
- policy deny 有结构化 report，最终可生成稳定、可行动的 Agent-facing response。

## 验证

使用本机 `cargo`，不要用会自动安装 toolchain 的 repo helper。

针对 context/policy 重构的最低验证：

```bash
cargo fmt --all -- --check
cargo check -p loong-core -p loong-access -p loong-kernel -p loong-app -p loong
cargo test -p loong-access
cargo test -p loong-kernel policy
cargo test -p loong-kernel access
cargo test -p loong-app file_read
cargo test -p loong-app workspace_root_tests
git diff --check
```

如果移动 crate 依赖，再跑：

```bash
./scripts/check_architecture_boundaries.sh
```
