# Tool Access / Action / Policy / Backend 临时计划

这是临时设计计划，不并入 `docs/`。本文按后续讨论后的形状更新：

- concrete unified context 由 app 定义；
- kernel 通过 context type factory 泛型连接 policy/access/tool；
- policy 绑定 context factory，不通过 `PolicyEngine` 间接拿 `Cx`；
- `PolicyEngine` 内不再定义或持有 factory / `Cx` associated type；
- unified context 本身替代 `ToolCoreContext`；tool/access/policy 都围绕同一个
  invocation context 工作，`ToolCoreContext` 是删除目标，不做长期 wrapper；
- tool 执行模型不再区分 CoreTool / ExtensionTool；tool 来源只作为注册 metadata
  保留，旧双 execution API 迁移时直接移除；
- migrated side effect 只能从 access 进入；
- 当前 object-safe action metadata trait 改名为 `ActionMeta`；
- 新的 executable action trait 是 `Action<Cx>: ActionMeta + Sized`，其 `run`
  直接接受 `Granted<Self>`；
- `Granted<A>` 提供 port，消费授权 token 后调用 `A::run(...)`；
- backend 不是必需抽象；如果后续存在，也只是 action run 内部实现细节，不能暴露给
  tool。

## 已确认原则

- 敢于破坏性改动是默认迁移策略。对已确认的新边界，优先一次性改调用点并删除旧
  类型/入口；不为已迁移路径保留 alias、proxy、compatibility shim 或 root re-export
  来拖延收口。只有明确对外兼容需求时，才把兼容层写成短期 migration item。
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
- unified context 是 invocation value，可以由 app 在调用链上派生出 narrowed /
  enriched 的新值；access/policy/action 观察时借用 `&ctx`，不能消费或隐藏替换 ctx。
- access 的 receiver 是 unified context。外部调用形状应是
  `ctx.access().fs().read_file(&path)`，不是 `kernel.access(ctx)`，也不是
  `ToolCoreContext::access()`。
- `ContextFactory` 在 kernel/policy/access/tool 之间作为显式泛型参数 `C` 传递，
  不作为 `Kernel` 的 associated type 再间接取用。
- `ContextFactory` 是 type-level factory，只有 GAT，没有 `create` / `build`
  method。context 构造发生在 app/runtime 边界，不发生在 core trait 里。
- `Policy` / `PolicyAny` 绑定 `ContextFactory`；`PolicyEngine` 不拥有 factory，
  也不定义 `type Cx` / `type Context`。
- `Policy` 和 `PolicyAny` API 应保持一致，例如 `name() -> Cow<'static, str>`。
- 业务 policy 不依赖 app concrete context；它应为任意满足所需 context requirement trait 的
  `C::Cx<'_>` 实现。后续代码注释要明确这点，避免 policy 偷偷绑定
  `AppExecutionContext`。
- 需要额外信息时，用小 context requirement trait 表达使用点需求，例如
  `FsAccessContext`、`ExecutionView`、`ToolInvocationView`。不要定义一个大 deps struct，也不要为每个
  domain 在 kernel 上加 getter。
- 只有极基础、跨所有 policy/access/tool 都成立的 context requirement 才放
  `loong-core`。需要 app/spec/test 实现、但服务于 kernel-governed access/policy
  路径的 requirement，由 `loong-kernel` 公共边界定义。app 不能为了实现 context
  requirement 直接依赖 `loong-access`；access crate 也不能反向依赖 kernel，所以这类
  requirement 不放在 access。名字按语义定，不强制 `View` 后缀，也不引入统一
  `context_view` 分类层。
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
- tool 管理参考 `/Users/yang/Projects/mvp` 的 typed tool 主干形状：具体工具是独立
  type + `impl ToolImpl<C>`，注册后擦除成 `RegisteredTool<C>`。但 loong 不继承 mvp
  里的第二套 `ToolContext` / `ToolHost` 包装；其位置由 unified context 顶上。
- 新增 tool 的改动面必须收敛到两处：一个 concrete type 的 `impl ToolImpl<C>`，以及
  一个 app/bootstrap/builtin 注册点（例如 `register(path, X)`，或带 metadata 的等价
  单次注册）。不能为了新增工具去改 `ToolPlane` match、dispatcher 分支、catalog 拼装
  分支或 policy preflight 分支。
- 新 `ToolPlane<C>` 属于 kernel runtime：它持有 tool registry、处理 lookup/invoke、
  连接 audit/error/provenance。`loong-core` 只放 `ToolImpl<C>`、`ToolRegistration`、
  `RegisteredTool<C>` 这类抽象和注册结果对象；`loong-contracts` 只放 `ToolSpec` /
  `ToolOutcome` 等稳定数据。
- 现有 `ToolPlane<C>` 实际是 legacy adapter plane，应先直接重命名为
  `LegacyToolPlane<C>`。不保留 `type ToolPlane = LegacyToolPlane` alias，也不引入
  `ToolAdapterPlane` 这种看似长期有效的新层名。`CoreToolAdapter` /
  `ToolExtensionAdapter` 只允许被 `LegacyToolPlane` 临时包住，后续随工具迁移一起删除。
- CoreTool / ExtensionTool 不再是目标 execution API。原 core/extension 差异只能作为
  provenance / registration metadata，用于 resolve、audit、catalog、namespace 和
  compatibility；迁移期旧 API 只能被隔离在 `LegacyToolPlane`，不能继续占用
  `ToolPlane` 这个正名，也不能被包装成新的 adapter plane 正常层。
- 迁移期允许一个明确 fallback：`Kernel::invoke_tool` 先查新 `ToolPlane`，没有 typed
  match 时跳到 `LegacyToolPlane`。fallback 必须记录为 legacy route，且只在未命中新
  registry 时发生；不能让新 `ToolPlane` 自己持有或调用 legacy plane。
- 需要 access 的具体 tool 通过 kernel 暴露的 context requirement 获取 `ctx.access()`。
  这个 trait 不放 `loong-access`，因为 app/spec/test 需要实现它但不应依赖 access；
  也不放 `loong-core`，因为它返回 kernel-defined `AccessCx`。
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
tool invocation
  -> kernel::ToolPlane<C>::resolve(path/name)
  -> RegisteredTool<C>::invoke(&ctx, payload)
  -> ToolImpl<C>::parse_input(payload)
  -> ToolImpl<C>::execute(&ctx, input)
  -> ctx.access()
  -> AccessCx<'_, '_, C>
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

unified context 是整个 invocation 的唯一 context。它可以是 owned value，并且 app 可以
通过派生方法生成更具体的 context：

```rust
let ctx = base_ctx.with_tool_invocation(...)?;
let ctx = ctx.with_runtime_narrowing(...)?;
let output = ctx.access().fs().read_file(&path).await?;
```

access facade 只借用 `&ctx`。`AccessCx::new(...)` 只应出现在具体
context 的 `access()` 实现里；其它调用点使用 `ctx.access()`：

```rust
impl<'a> AppExecutionContext<'a> {
    pub(crate) fn access(&self) -> AccessCx<'_, 'a, AppContextFactory> {
        AccessCx::new(self.kernel, self)
    }
}
```

`AccessCx` 可以持有 kernel ref 来取得 policy engine / runtime host；ctx 提供的是
invocation 数据和 app-facing requirement。外部 API 的 receiver 仍是 unified ctx，
不是 `kernel.access(ctx)`。

tool execution API 是单一的：

```rust
#[async_trait]
pub trait ToolImpl<C: ContextFactory>: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + Into<ToolOutcome> + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: serde_json::Value) -> Result<Self::Input, ToolInputError>;

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError>;
}
```

具体工具是单独类型实现这个 trait，不写进 `ToolPlane` 的 match/enum：

```rust
pub struct ReadFileTool;

pub struct ReadFileInput {
    path: String,
}

pub struct ReadFileOutput {
    path: String,
    bytes: usize,
    content: String,
}

impl From<ReadFileOutput> for ToolOutcome {
    fn from(output: ReadFileOutput) -> Self {
        ToolOutcome {
            status: "ok".to_owned(),
            payload: serde_json::json!({
                "path": output.path,
                "bytes": output.bytes,
                "content": output.content,
            }),
        }
    }
}

#[async_trait]
impl<C> ToolImpl<C> for ReadFileTool
where
    C: ContextFactory,
    for<'a> C::Cx<'a>: KernelAccess<C>,
{
    type Input = ReadFileInput;
    type Output = ToolOutcome;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".to_owned(),
            description: "Read a file from the allowed filesystem roots.".to_owned(),
            required_capabilities: vec![Capability::FilesystemRead],
        }
    }

    fn parse_input(&self, payload: serde_json::Value) -> Result<Self::Input, ToolInputError> {
        let path = payload
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or(ToolInputError::MissingField("path"))?
            .to_owned();
        Ok(ReadFileInput { path })
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        let output = ctx.access().fs().read_file(&input.path).await?;
        let content = String::from_utf8_lossy(&output.bytes).to_string();
        Ok(ReadFileOutput {
            path: output.path.display().to_string(),
            bytes: output.bytes.len(),
            content,
        }
        .into())
    }
}
```

`ToolImpl` 本身在 core，`ReadFileTool` 这类 concrete impl 放 app 或后续 builtin-tools
crate。需要 access 时约束 kernel 暴露的 `KernelAccess<C>`（命名可实现时再收敛），
不依赖 `loong-access` 的 trait。

新增工具时只能新增/修改两处：具体工具类型的 `impl ToolImpl<C>`，以及 app/bootstrap
或 builtin registration 位置的一条注册记录。注册记录可以携带 path、metadata、
provenance 或 catalog 信息，但它必须仍是“一次注册”，而不是把工具散落到多个
dispatcher/catalog/policy 分支里。

`kernel::ToolPlane<C>` 持有 `RegisteredTool<C>`，注册记录携带 `registered_at` 和来源
metadata。来源 metadata 可以区分 builtin / extension / discovered / compatibility route，
但 `RegisteredTool::invoke(&ctx, payload)` 只有一条路径。

迁移到这条路径时应先把旧调用面隔离成 `LegacyToolPlane<C>`。`CoreToolAdapter` /
`ToolExtensionAdapter` 只能短期服务未迁移工具，并随 legacy plane 删除；新
`ToolPlane<C>` 不走 adapter。过渡期间 fallback 由 `Kernel::invoke_tool` 执行：新
registry 未命中时才转交 legacy plane。

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
- `ToolSpec`
- `ToolOutcome`
- `ToolPath`，或先用现有 string tool id 作为过渡路径类型
- `ToolInputError` 这类纯输入错误数据
- legacy tool/runtime/memory request/outcome 数据结构

不放：

- `ContextFactory`
- `PolicyContext` / 其它 context requirement traits
- `ActionMeta`
- `Action<Cx>`
- `Policy`
- `PolicyAny`
- `PolicyEngine`
- `ToolImpl<C>`
- `RegisteredTool<C>`
- `ToolPlane<C>`
- workspace root / file root / tool config 视图

这些是行为 trait 或 context requirement trait，属于 `loong-core`、`loong-kernel`
或更具体的公共边界。需要 app 实现的 access-backed requirement 不属于
`loong-access`，因为 app 不依赖 access。

### `loong-core`

放跨 kernel/access/policy/tool 共用的行为 contract。core 承载极基础的 context
requirement，例如 `ContextFactory` 和 capability gate 所需的最小 `PolicyContext`
形状；也承载不依赖 kernel runtime 的 tool 抽象与单工具注册对象。不要把
`FsAccessContext` 这类 domain-specific access requirement 放进 core：

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
pub struct ActionMetadata<'a> {
    pub kind: &'static str,
    pub operation: Cow<'a, str>,
    pub required_capabilities: Cow<'a, [Capability]>,
}

pub trait ActionMeta {
    fn metadata(&self) -> ActionMetadata<'_>;

    fn audit_resource(&self) -> Option<Cow<'_, str>>;

    fn payload(&self) -> serde_json::Value;
}
```

`metadata()` 只返回便宜、可借用的授权/audit 元信息；`payload()` 是按需构造的
JSON 视图，供 `PolicyAny` 等 type-erased policy 使用。不要命名为
`legacy_*` 或 `policy_*`：Action 类型本身已经表达 policy 语义，payload 只是
该 Action 的结构化载荷。`payload()` 没有默认值；每个 Action 必须显式声明
自己的 type-erased payload。

Executable action 绑定到具体 context，并且 `run` 直接接受 `Granted<Self>`：

```rust
pub trait Action<Cx>: ActionMeta + Sized {
    type Output;
    type Error;

    async fn run(granted: Granted<Self>, cx: &Cx) -> Result<Self::Output, Self::Error>;
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

tool 抽象也放在 core，但只到单个工具的注册和 erased invoke，不拥有运行平面：

```rust
#[async_trait]
pub trait ToolImpl<C: ContextFactory>: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + Into<ToolOutcome> + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: serde_json::Value) -> Result<Self::Input, ToolInputError>;

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError>;
}

pub struct ToolRegistration {
    spec: ToolSpec,
    registered_at: SystemTime,
    provenance: ToolProvenance,
}

pub struct RegisteredTool<C: ContextFactory> {
    registration: ToolRegistration,
    erased: Box<dyn ErasedTool<C>>,
}
```

`RegisteredTool<C>` 是一个工具的注册结果对象；它可以擦除具体 `ToolImpl<C>` 并提供
`invoke(&C::Cx<'_>, Value)`。不要在 core 里放 `BTreeMap<ToolPath, RegisteredTool<C>>`
这种 runtime registry，也不要让 core 处理 audit、pack、namespace 或 adapter fallback。

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
policy 构造参数或小 context requirement trait 表达。

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

`PolicyContext` 是最小 root context requirement，主要服务 capability gate。旧 `ActionContext`
已删除；`ExecutionPlane` / `PlaneTier` 仍可作为 invocation/global execution fact
保留在具体 context 里。后续若有 policy/access 需要读取它们，再拆成更准确的小
context requirement trait，例如 `ExecutionView`，只在真正需要的位置约束。只有当某个
requirement 确实是跨所有治理路径的基础能力时，才提升到 core。

### `loong-kernel`

kernel 固定治理流程，不固定 app context 字段。

目标形状：

```rust
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    tool_plane: ToolPlane<C>,
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

新 `ToolPlane<C>` 也属于 kernel。它拥有一组 core `RegisteredTool<C>`，负责 runtime
lookup、执行入口、错误转换、provenance/audit 接入：

```rust
pub struct ToolPlane<C: ContextFactory> {
    tools: BTreeMap<ToolPath, RegisteredTool<C>>,
}

impl<C: ContextFactory> ToolPlane<C> {
    pub fn register<T>(&mut self, path: ToolPath, tool: T) -> Result<(), ToolPlaneError>
    where
        T: ToolImpl<C>,
    {
        let registered = RegisteredTool::from_tool(ToolProvenance::Builtin, tool)?;
        self.tools.insert(path, registered);
        Ok(())
    }

    pub async fn invoke(
        &self,
        path: &ToolPath,
        ctx: &C::Cx<'_>,
        payload: serde_json::Value,
    ) -> Result<ToolOutcome, ToolPlaneError> {
        let registered = self
            .tools
            .get(path)
            .ok_or_else(|| ToolPlaneError::ToolNotFound(path.to_string()))?;
        registered.invoke(ctx, payload).await.map_err(ToolPlaneError::from)
    }
}
```

迁移期间如果未迁移工具还需要旧 adapter 调用面，旧 `ToolPlane<C>` 只能先改名为
`LegacyToolPlane<C>`，并标注为删除对象：

```rust
pub struct LegacyToolPlane<C: ContextFactory> {
    core_adapters: BTreeMap<String, Arc<dyn CoreToolAdapter<C>>>,
    extension_adapters: BTreeMap<String, Arc<dyn ToolExtensionAdapter<C>>>,
    default_core_adapter: Option<String>,
}
```

迁移期 `Kernel<C>` 可以临时持有 `legacy_tool_plane: LegacyToolPlane<C>`，但目标
`Kernel<C>` 只保留新 `tool_plane: ToolPlane<C>`。旧 `register_core_tool_adapter` /
`execute_tool_core` 这类 kernel API 可以暂留一小步，但注释要明确它们是 legacy path，
并在迁移完成后删除。新工具注册走 `Kernel::register_tool(path, tool)`，新工具执行走
`Kernel::invoke_tool(path, payload, ctx)` 或等价命名。

fallback 放在 kernel，不放在新 `ToolPlane`：

```rust
impl<C: ContextFactory> Kernel<C> {
    pub async fn invoke_tool(
        &self,
        path: &ToolPath,
        payload: serde_json::Value,
        ctx: &C::Cx<'_>,
    ) -> Result<ToolOutcome, KernelError> {
        match self.tool_plane.invoke(path, ctx, payload.clone()).await {
            Ok(outcome) => Ok(outcome),
            Err(ToolPlaneError::ToolNotFound(_)) => {
                self.record_legacy_tool_fallback(path)?;
                self.legacy_tool_plane
                    .execute_core_with_context(None, legacy_request(path, payload), ctx)
                    .await
                    .map(Into::into)
                    .map_err(KernelError::from)
            }
            Err(error) => Err(KernelError::from(error)),
        }
    }
}
```

这段 fallback 是迁移期脚手架：只处理 typed registry 未命中，不覆盖 typed tool 的错误，
并且必须有 audit/provenance 记录表明本次执行走了 legacy plane。

`PolicyPipeline` 的语义固定：

- `pre` stage：broad gates，先于 typed action policy；
- `action` stage：按 concrete action type 注册；
- `fallback` stage：兼容旧 action 的 broad policy；
- `Allow` / `Deny` 终止整个 pipeline；
- `Continue` 继续当前 subchain；
- `Advance` 跳到下一个 subchain；
- 没有 terminal decision 时 default deny；
- `PolicyReport` 记录完整 evaluations 和 outcome。

`Kernel<C>` 是把 policy engine、tool registry、access facade 统一到同一个
`C::Cx<'a>` 的地方。它不应该有 `fs_policy_context()` / `browser_policy_context()`
这类 domain getter，也不应该发明第二套 tool context wrapper。
kernel-owned governance context requirement 属于 kernel，例如
`KernelInvocationContext`。它不应上移到 core；后续若需要整理文件，可以放到
kernel-owned module，但不需要统一叫 view。

当前代码状态：`Kernel<C>`、`PolicyPipeline<C>`、`AccessCx<'a, 'ctx, C>`、
`loong_core::tool::{ToolImpl, RegisteredTool}`、kernel typed `ToolPlane<C>`、
legacy `LegacyToolPlane<C>` 已落地。`Kernel::invoke_tool` 先查 typed
`ToolPlane<C>`，只有 `ToolPlaneError::ToolNotFound` 才转 legacy plane；typed tool
自己的 parse/execute 错误不会 fallback。`ToolCoreContext` 已删除，tool adapter 直接接收
`&C::Cx<'_>`。kernel 不再定义 `KernelPolicyContext` / `KernelContextFactory`，也不再为
`Kernel`、legacy plane 或 `PolicyPipeline` 提供默认 context factory。app 和 spec
分别定义自己的 concrete context factory。

### `loong-access`

access crate 定义 side-effect domain 的治理入口和 action。

以 fs 为例：

- `FsAccess`
- `FsReadAction`
- `FsAction`
- `CanonicalPath`
- `FsReadOutput`

`loong-access` 不定义需要 app/spec/test 直接实现的 context requirement。否则 app
会被迫依赖 access。fs root 这类 access-backed 执行前提应由 kernel 公共边界定义，
kernel facade 从 unified context 读取后，把普通数据交给 access：

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

字段只是示意；关键是 concrete context 由 app 拥有。它按需实现小 context requirement trait：

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
应该把配置作为自己的显式构造参数、字段或小 context requirement trait 依赖；policy impl 仍然要
对任意满足这些 trait 的 context 生效，而不是绑定 app concrete context。

## 当前执行状态

### 已完成

- `loong-access` 已创建，依赖 `loong-core` 和 `loong-contracts`。
- `loong-access` 当前只被 `loong-kernel` 依赖，app 不直接依赖 access；这是长期边界，
  不是临时实现细节。
- `FsReadAction` / `FsAction` / `CanonicalPath` / `FsAccess::read_file` 已落地。
- `file.read` / `read` path mode 已迁移到 access-backed read；当前调用经由
  `ctx.access().fs().read_file(...)`。
- migrated read 已退出 `direct_policy_preflight` 的 file 分支。
- `FilePolicyExtension` 不再覆盖 read，暂时只服务未迁移的 file surfaces。
- `PolicyPipeline` 已有 typed registry、pre/action/fallback stages、`PolicyReport`。
- `ContextFactory` 已在 `loong-core` 落地，只有 GAT，没有 create/build method。
- `Policy` / `PolicyAny` / `PolicyEngine` 已改为显式 `C: ContextFactory` 泛型。
- `PolicyEngine` 不再定义 `type Cx`。
- `Kernel<C>` / `PolicyPipeline<C>` 已泛型化。
- 旧 adapter plane 已直接改名为 `LegacyToolPlane<C>`，没有保留
  `type ToolPlane = LegacyToolPlane` alias，也没有引入 `ToolAdapterPlane`。
- `loong-contracts` 已增加 typed tool 纯数据：`ToolPath`、`ToolSpec`、
  `ToolOutcome`、`ToolInputError`、`ToolExecutionError`。
- `loong-core::tool` 已增加 `ToolImpl<C>`、`ToolRegistration`、
  `RegisteredTool<C>`，并把 erased invoke 限定为 core-private `ErasedTool<C>`。
- `loong-kernel::tool::ToolPlane<C>` 已增加 path -> `RegisteredTool<C>` registry；
  runtime registry 没有放进 core。
- `Kernel<C>` 已同时持有 typed `tool_plane: ToolPlane<C>` 和
  `legacy_tool_plane: LegacyToolPlane<C>`。
- `Kernel::invoke_tool` 已实现 typed-first 调用；只有 typed registry miss
  (`ToolPlaneError::ToolNotFound`) 才 fallback 到 `LegacyToolPlane<C>`，并用 audit
  metadata 标出 `typed-tool-plane` 或 `legacy:{adapter}` route。
- app 已把 `ReadFileTool` 做成独立 `ToolImpl<AppContextFactory>`，并在统一的
  kernel tool registration 入口注册到 typed `ToolPlane<C>`。
- app kernel tool 请求已切到 `Kernel::invoke_tool`；`read/file.read` path mode
  命中 typed plane，direct `read` 的 query/glob 模式仍在迁移期路由到 legacy
  `content.search` / `glob.search`。
- `ToolCoreContext` 已删除；kernel/tool/app 直接传递 app/spec/test 定义的 unified
  context。
- kernel facade `AccessCx<'a, 'ctx, C>` 只保留 context factory 泛型，不再暴露
  额外 `K: Kernel` 参数；它借用 unified context，不 own context。
- `Kernel<C>` 构造函数已泛型化，可以实例化非默认 context factory。
- `ActionContext` / `WorkspacePolicyContext` 已从 `loong-core` 删除。
- `KernelPolicyContext` / `KernelContextFactory` 已从 kernel 删除；legacy kernel
  policy extension 只通过 `KernelInvocationContext` 小 context requirement trait 读取 pack/token/time/
  request params。
- app 已定义 `AppContextFactory` / `AppExecutionContext<'a>`，并实现
  `PolicyContext`、`KernelInvocationContext`、`FsAccessContext`。
- spec 已定义 `SpecContextFactory` / `SpecExecutionContext<'a>`，用于 spec bootstrap
  和 daemon/spec runtime。
- `ToolCoreContext::with_fs_root_view(...)` 已删除；fs root view 在 app context 构造时
  给出。
- `AccessCx` / `FsAccess` 已改为借用 `&ctx`，外部调用点不再使用
  `kernel.access(ctx)`。
- fs read execution boundary 已收敛到 `Action<Cx>::run` /
  `Granted<A>::run(ctx)`。
- `PolicyDecision` 已是 `Allow` / `Deny` / `Continue` / `Advance`。
- `deny_read_filenames` 已作为 typed `FsReadAction` policy 接入。
- `deny_read_filenames` 已有 config -> runtime config -> policy pipeline ->
  typed policy registration 路径。

### 仍是过渡形状

- `FsAccessContext` 仍在 `loong_core::policy::context`；长期不应留在 core，也不能迁到
  `loong-access` 迫使 app 依赖 access。目标是由 kernel 公共边界定义 app-facing fs
  context requirement，kernel facade 抽取 `resolution_root` / `allowed_roots` 后把
  普通数据传给 access。
- `CoreToolAdapter` / `ToolExtensionAdapter` 和 `execute_tool_core` /
  `execute_tool_extension` 仍是 legacy execution API；它们只服务未迁移工具，后续迁移到
  单一 typed tool execution path 后删除。
- `ToolCoreRequest` / `ToolExtensionRequest` 仍在 contracts/kernel 路径中扩散；长期应
  收敛为统一 tool invocation 数据，core/extension 只保留为 provenance metadata。
- `loong_access::fs::FsAccess` 内部只持有 policy engine 引用，不再持有 kernel host。
  该类型仍有 `P: PolicyEngine<C>` 泛型，因为 `PolicyEngine<C>` 需要 generic
  action grant，不能直接做成普通 trait object。
- `fs_read_error_is_policy_denial` 仍是临时 deny 分类 helper。
- HTTP / shell / browser / memory 等 tool family 尚未迁移。
- child action constructor pattern 尚未落地。
- `PolicyReport` 尚未贯穿 access/tool error 到 Agent-facing response。

## 实现计划

### 1. 收敛 context type factory（已完成）

已在 `loong-core` 增加只有 GAT 的 `ContextFactory`：

```rust
pub trait ContextFactory {
    type Cx<'a>: PolicyContext
    where
        Self: 'a;
}
```

本阶段已清理：

- `ActionContext` root bound；
- `WorkspacePolicyContext`；
- kernel-owned `KernelPolicyContext` / `KernelContextFactory`。

`ContextFactory` 不提供 `create` / `build` method。app 自己构造 concrete
context，factory 只提供类型族。

除 `ContextFactory` 自己的 GAT 外，`C` 一律作为泛型参数显式传递，不通过
`Kernel::C` / `Kernel::ContextFactory` 这类 associated type 镜像：

```rust
PolicyEngine<C>
Policy<C, A>
PolicyAny<C>
Kernel<C>
AccessCx<'a, 'ctx, C>
ToolPlane<C>
RegisteredTool<C>
```

`loong_access::fs::FsAccess` 内部保留 `P: PolicyEngine<C>` 泛型；access crate 不依赖
concrete `loong_kernel::Kernel<C>`，也不持有整个 kernel host。

### 2. 拆 ActionMeta / Action<Cx>（已完成）

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
  - async run(Granted<Self>, &Cx)
```

`Granted<A>` 增加 port：

```rust
Granted<A>::run<Cx>(&Cx).await
where
    A: Action<Cx>
```

`ActionMeta` 的 docs/comment 要说明它只是 policy/audit/type-erased metadata view；
便宜元信息走 `metadata()`，动态 JSON 载荷走 `payload()`。
`Action<Cx>` 的 docs/comment 要说明它是 side-effect implementation hook，并且
`run` 必须消费 `Granted<Self>`。`Granted<A>::run(cx)` 的 docs/comment 要说明它是
授权 token 到执行的推荐入口。

已落地：`ActionMeta` 只作为 policy/audit metadata view；`Action<Cx>` 是 async
side-effect hook；`FsReadAction` 的读取副作用经由 `Granted<FsReadAction>::run(&ctx)`
进入，不再保留平行的 `read_granted_file(...)` 执行函数。`ActionMeta` 已收敛为
`metadata()` + `payload()`：`metadata()` 不分配 capability set，`payload()`
服务 type-erased policy 的结构化输入。

### 3. 改 Policy / PolicyAny / PolicyEngine 泛型（已完成）

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

已落地：`loong-core` 中的 `Policy<C, A>` / `PolicyAny<C>` / `PolicyEngine<C>` 已统一
使用显式 `C: ContextFactory` 泛型；`kernel::PolicyPipeline<C>` 和 typed/any policy
registry 也使用同一个 `C`，`PolicyEngine` 内没有 factory 或 `Cx` associated type。

### 4. 用 unified context 替换 ToolCoreContext（已完成）

把 concrete kernel 迁到：

```rust
pub struct Kernel<C: ContextFactory> {
    policy: PolicyPipeline<C>,
    // ...
}
```

删除 `ToolCoreContext`。tool adapter/helper 看到的 context 就是 `C::Cx<'_>` 本身，
不是 kernel 再包装出的二级 context。tool-facing adapter wrapper 不再需要；旧 adapter
路径只隔离在 `LegacyToolPlane<C>`，新 typed path 的 erased invoke 放在
core-private `ErasedTool<C>` 里。

已落地：`ToolCoreContext` wrapper 已删除；`CoreToolAdapter::execute_core_tool_with_context`
直接接收 `&C::Cx<'_>`；access-backed read path 通过
`ctx.access().fs().read_file(...)` 进入。

单一 typed tool execution API 的骨架已落地；`read/file.read` 已作为第一条 concrete
tool path 注册并调用 typed plane。仍待迁移的是其它 concrete tool 和旧 adapter API
删除。

第一步已落地：旧 adapter plane 已直接改成 legacy 名称，给目标 `ToolPlane` 腾出语义空间：

```rust
pub struct LegacyToolPlane<C: ContextFactory> {
    core_adapters: BTreeMap<String, Arc<dyn CoreToolAdapter<C>>>,
    extension_adapters: BTreeMap<String, Arc<dyn ToolExtensionAdapter<C>>>,
    default_core_adapter: Option<String>,
}
```

迁移期 `Kernel<C>` 字段改为 `legacy_tool_plane: LegacyToolPlane<C>`。旧
`register_core_tool_adapter` / `register_tool_extension_adapter` / `execute_tool_core`
暂时保留，但 rustdoc 写明它们是 legacy path。不要保留
`type ToolPlane = LegacyToolPlane`，也不要引入 `ToolAdapterPlane`。

第二步已落地：contracts/core 已增加 typed tool 抽象。公开给具体工具实现者的是
`ToolImpl<C>`；`RegisteredTool<C>` 内部做 payload parse 和 erased invoke：

```rust
#[async_trait]
pub trait ToolImpl<C: ContextFactory>: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + Into<ToolOutcome> + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: serde_json::Value) -> Result<Self::Input, ToolInputError>;

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError>;
}
```

具体工具必须是单独 type + 单独 trait impl，不能写进 `ToolPlane` 的 match：

```rust
pub struct ReadFileTool;

#[async_trait]
impl<C> ToolImpl<C> for ReadFileTool
where
    C: ContextFactory,
    for<'a> C::Cx<'a>: KernelAccess<C>,
{
    type Input = ReadFileInput;
    type Output = ToolOutcome;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".to_owned(),
            description: "Read a file from the allowed filesystem roots.".to_owned(),
            required_capabilities: vec![Capability::FilesystemRead],
        }
    }

    fn parse_input(&self, payload: serde_json::Value) -> Result<Self::Input, ToolInputError> {
        let path = payload
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolInputError::MissingField("path"))?
            .to_owned();
        Ok(ReadFileInput { path })
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, ToolExecutionError> {
        let output = ctx.access().fs().read_file(&input.path).await?;
        Ok(ReadFileOutput::from(output).into())
    }
}
```

第三步已落地：kernel 已增加新的 typed `ToolPlane<C>`。它只维护 path -> core
`RegisteredTool<C>` 的 runtime registry；provenance metadata 通过
`RegisteredTool<C>::registration()` 读取：

```rust
pub struct ToolPlane<C: ContextFactory> {
    tools: BTreeMap<ToolPath, RegisteredTool<C>>,
}
```

`ToolProvenance` 可以表达 builtin / extension / discovered / compatibility route，但
`RegisteredTool::invoke(&ctx, payload)` 只有一条 execution path。

第四步已落地：fallback 接在 kernel 层。`Kernel::invoke_tool` 先调用新
`ToolPlane<C>::invoke`，只有 `ToolPlaneError::ToolNotFound` 才转
`LegacyToolPlane<C>`。typed tool 自己 parse/execute 失败时不能 fallback，否则会掩盖新
工具错误。

`read/file.read` concrete tool 迁移已落地：app 定义 `ReadFileTool`，注册进 typed
`ToolPlane<C>`，app/kernel 调用点走 `Kernel::invoke_tool`。后续应按同一模式迁移其它
concrete tool；完成迁移后删除 `LegacyToolPlane<C>`、`CoreToolAdapter`、
`ToolExtensionAdapter` 和旧 `execute_tool_core` / `execute_tool_extension` API。
不能把 legacy fallback 留作长期路径。

`AccessCx` 显式携带 `C`，但它由 `ctx.access()` 构造，并且只借用 unified context。
如果具体 tool 需要 access，它通过 kernel 暴露的 context requirement trait 获取：

```rust
pub trait KernelAccess<C: ContextFactory>: PolicyContext {
    fn access(&self) -> AccessCx<'_, '_, C>;
}
```

具体命名可在实现时按当前模块收敛；关键是不让 trait 进 `loong-access`，也不让
`Kernel` 成为 access 的外部 receiver。工具调用保持：

```rust
ctx.access().fs().read_file(&path).await
```

### 5. App 落地 concrete unified context

已在 app 定义：

- `AppContextFactory`
- `AppExecutionContext<'a>`
- 所需 context requirement traits 的 impls

注意：concrete context 由 app 定义，但 context requirement trait 不一定定义在 app。
requirement 的定义位置跟随依赖边界：

- `PolicyContext` 这类极基础、跨治理路径的 requirement 可以放在 `loong-core`；
- `FsAccessContext` 这类 access-backed 前提如果需要 app 实现，就由 `loong-kernel`
  公共边界定义，kernel 再把提取出的普通数据交给 `loong-access`；
- `KernelInvocationContext` 这类 kernel governance requirement 也属于 `loong-kernel`；
- 只有 app 私有、且不会成为跨 crate 约束的 requirement 才放在 app。

app 的职责是把 `AppExecutionContext<'a>` 实现为这些 requirement 的并集。
`SpecExecutionContext` 或测试 context 也可以实现同一组或子集 requirement；policy/access
只能通过 trait bound 读取需要的信息，不能绑定 app concrete context。

当前代码仍是过渡状态：`PolicyContext` / `FsAccessContext` 在
`loong_core::policy::context`，`KernelInvocationContext` 在 `kernel::policy`。后续应
做一次最小移动：只保留极基础 requirement 在 core，把 `FsAccessContext` 这类
app-facing access-backed requirement 移到 kernel 公共边界，直接更新调用点，不保留
root alias。

`KernelPolicyContext::with_fs_root_view(...)` 已删除。fs root view 现在由 app context
构造处提供；fs root view 不由 action 持有，也不由 access helper 临时塞入 kernel
context。

### 6. 固化 Config -> Policy 组装边界

保留并推广当前 `deny_read_filenames` 的方向：

- config parsing 只产出 config 数据；
- runtime config 做 normalization / narrowing；
- app context/bootstrap 根据 runtime config 构造并注册 policies；
- kernel pipeline 只接收已构造好的 policy；
- access/tool helper 不反向读取 app config 来决定授权。

引入 `AppContextFactory` 后，config 仍不进入 `ContextFactory` trait 本身。
factory 只提供 context type family；policy pipeline construction 可以读取 normalized
runtime config 来构造 policy。

### 7. 清理 legacy policy/context 形状

删除或替换：

- 当前 object-safe `Action` metadata trait；
- `Policy<PolicyPipeline, A>` 这种 engine-bound policy impl；

已完成：

- `ActionContext` root bound 已删除；
- `WorkspacePolicyContext` 已删除。
- `KernelPolicyContext` / `KernelContextFactory` 已删除；
- `ToolCoreContext::with_fs_root_view(...)` 已删除；
- kernel/app/spec/daemon 调用点已显式传入 app/spec/test context。
- `ToolCoreContext` wrapper 本身已删除；tool adapter/helper 直接接收
  `&C::Cx<'_>`。

仍待删除：

- `CoreToolAdapter` / `ToolExtensionAdapter` 双 execution API；
- `execute_tool_core` / `execute_tool_extension` 双 kernel entry；
- `ToolCoreRequest` / `ToolExtensionRequest` 在主 execution path 上的扩散。

保持破坏性改动优先，不为已迁移路径保留 alias / compatibility shim。

### 8. 优化 deny/report 路径

把 policy denial 作为结构化 authorization error 贯穿：

```text
PolicyReport
  -> PolicyGrantError
  -> FsAccessError
  -> ToolPlaneError / ToolOutcome
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
- tool-facing API 暴露 policy context、domain context requirement trait、backend handle。
- tool-facing API 暴露 `ToolCoreContext` 或第二套 tool context wrapper。
- 外部 access 调用以 `kernel.access(ctx)` 或 wrapper `.access()` 为主语；access 的
  主语必须是 unified ctx。
- 新 typed tool path 继续区分 CoreTool / ExtensionTool 两套 execution API；来源差异只能是
  metadata。
- 旧 adapter path 继续叫 `ToolPlane`，或者为迁移方便保留
  `type ToolPlane = LegacyToolPlane` alias。
- 把旧路径命名成 `ToolAdapterPlane` 这类看似长期有效的正常层。
- 在完成 typed tool 迁移后继续保留 `LegacyToolPlane` / `CoreToolAdapter` /
  `ToolExtensionAdapter`。
- typed registry 已命中后仍 fallback 到 legacy，或用 legacy fallback 掩盖 typed tool 的
  input/execute 错误。
- 在 `ToolPlane` 里用 match/enum 写具体工具逻辑；具体工具必须是独立 type 的
  `ToolImpl<C>`。
- 新增工具时需要同步修改 dispatcher match、catalog builder、manual allowlist 或
  direct preflight 分支；这些都应由注册 metadata 和 typed policy/action 表达。
- 需要 access 的具体 tool 依赖 `loong-access` trait；tool 应依赖 kernel 暴露的
  `ctx.access()` requirement。
- `Policy` 通过 `PolicyEngine` 获取 `Cx`。
- `PolicyEngine` 定义 `type Cx` / `type Context`。
- `ContextFactory` 带 create/build method。
- `ActionMeta` 携带 execution output/error/run。
- `Action<Cx>` 不消费 `Granted<Self>` 就执行副作用。
- policy impl 依赖 `AppExecutionContext` 这类 app concrete context，而不是依赖小
  context requirement trait。
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
- concrete app context 由 app 定义并实现所需 context requirement traits。
- 业务 policy 为任意满足所需 context requirement trait 的 `C::Cx<'_>` 实现，不绑定
  `AppExecutionContext`。
- kernel 只通过泛型连接统一 context，不固定 app context 字段。
- `ToolCoreContext` 已删除；tool impl / registered tool / access 直接使用 unified
  ctx。
- 旧 adapter plane 已改名为 `LegacyToolPlane<C>`，没有 `ToolPlane` alias，也没有
  `ToolAdapterPlane` 新层。
- `loong-core` 提供 `ToolImpl<C>` / `ToolRegistration` / `RegisteredTool<C>`；
  `loong-contracts` 提供 `ToolSpec` / `ToolOutcome` 等纯数据。
- `loong-kernel` 提供新的 `ToolPlane<C>`，它只有单一 `RegisteredTool<C>` execution
  path；core/extension 来源只出现在 registration/provenance metadata。
- 迁移期 fallback 只发生在 typed registry 未命中时，并记录 legacy route；typed tool
  自身错误不触发 fallback。
- 具体工具是独立 type + `impl ToolImpl<C>`，不写进 `ToolPlane` match。
- 新增 tool 的正常 diff 只有两类：`impl ToolImpl<C> for X`，以及一个
  app/bootstrap/builtin 注册点；注册点可以带 metadata，但不能要求额外 dispatcher 或
  catalog 分支。
- 需要 access 的具体工具通过 kernel 暴露的 `ctx.access()` requirement 约束 context，
  不依赖 `loong-access` trait。
- `CoreToolAdapter` / `ToolExtensionAdapter` 不再是主执行抽象；它们只能临时存在于
  `LegacyToolPlane`，并在迁移完成后删除。
- `file.read` / `read` path mode 通过独立 `ReadFileTool` 的 `ToolImpl` 路径进入
  `ctx.access().fs().read_file(...)`，不经过 legacy adapter 执行读取。
- migrated side effect 不经过 direct preflight / `FilePolicyExtension`。
- config-driven policy 经由 app config -> normalized runtime config ->
  app-owned policy construction -> policy registration 进入 pipeline。
- `Granted<A>` 仍不可被 `loong-core` 外部伪造。
- 代码 comment/docs 说明 `ActionMeta` / `Action<Cx>` 分工，以及
  `Granted<A>::run(cx)` 的执行边界语义；`ActionMeta::payload()` 是 Action 的
  结构化载荷，不是 legacy-only bridge，且没有默认 `Null`。
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
