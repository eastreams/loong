# Tool Access / Action / Policy Plan

这是临时迁移计划，不并入 `docs/`。当前路线以 Access-Action-Policy 为主，不再让旧
authorization / adapter workaround 塑形新架构。

## 已确认原则

- 敢于破坏性改动。新边界确认后，直接迁移调用点并删除旧入口；不保留 alias、
  proxy、root re-export 或长期 fallback。
- concrete unified context 由 app 定义。kernel/access/policy/tool 只通过
  `ContextFactory` 和小的 context requirement trait 观察它。
- tool/access/policy 共享同一个 invocation context。旧 `ToolCoreContext` 是删除目标。
- 副作用 only access can do。已迁移 tool/helper/adapter/kernel policy 都不能直接执行
  文件读取等 migrated side effect。
- `ActionMeta::required_capabilities` 是 action 属性；workspace root、file root、
  runtime config 是 context / resolver / policy 的输入，不塞进 required caps。
- `Granted<A>` 是授权到执行的边界。没有 grant 就不能进入对应 side-effect 或 dispatch
  入口。
- policy 不依赖 app concrete context。需要 context 数据时，用小 requirement trait
  表达，例如 fs root view；业务 policy 应为任意满足 trait 的 context 实现。
- config -> policy 路径属于 app bootstrap：app 读取 config，构造 typed policy，注册进
  pipeline。access 不读取 app config，tool helper 不做 policy preflight。
- `path` 是 ToolPlane registry 的路径，不是 contracts/core 的全局概念。具体 path
  类型由具体 `ToolPlane` 定义；core 不能替所有 plane 规定 `ToolPath` 的结构。
- `ToolImpl` 不拥有 path。tool 自身只描述输入/输出/能力/说明；注册到某个 plane 时，
  plane 才把自己的 `Path` 和 tool descriptor 组合成 registered spec。
- `ToolInvocationAction` 可以存在，但它属于具体 plane/app 的治理边界，不能在 core 里
  持有全局 `ToolPath`。如果保留公共 helper，也必须是 `ToolInvocationAction<P>`，其中
  `P` 是具体 plane path。
- `ActionMeta::payload` 没有默认 `Null`。payload 是 Action 的结构化载荷；如果 action
  已经持有 `serde_json::Value`，目标 API 可以返回 `Cow<'_, Value>` 来避免无意义 clone。
- 架构代码要有少量高信号注释，标明边界和意图。注释解释 why，不重复代码，也不写大段
  散文。

## 分层边界

- `loong-contracts`：稳定数据类型，例如 `ToolOutcome`、`ToolInputError`、
  `AuditEventKind`、`PolicyReport`。不要定义全局 `ToolPath`；不要让 tool descriptor
  携带 registry path。
- `loong-core`：行为 trait 和不可伪造授权模型，例如 `ActionMeta`、`Action<Cx>`、
  `Granted<A>`、`ToolImpl<C>`、`RegisteredTool<C>`。core 不决定 ToolPlane path 类型；
  core 里的 registered tool 只擦除 concrete tool，不表达注册位置。
- `loong-kernel`：governance authority，提供 policy pipeline、token/pack boundary、
  grant、audit sink、clock/event id。kernel 不持有 typed tool registry，也不拥有
  concrete ToolPlane path 类型。
- `loong-access`：domain side-effect boundary。文件读取只在 fs access/action 路径中
  发生。
- `loong-app`：concrete context、config -> policy wiring、app-owned `ToolPlane`、
  concrete tool registration、legacy fallback orchestration。

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
trait ToolPlane<C: ContextFactory> {
    type Path: Clone + Ord;
}

struct AppToolPlane<C> {
    tools: BTreeMap<AppToolPath, RegisteredTool<C>>,
}
```

`AppToolPath` 可以是 `Vec<String>`、smallvec、interned path、trie key，或后续其它形状。
这是 app plane 的 registry path decision，不是 contracts/core 的决定。

tool 自身返回无 path descriptor：

```rust
pub struct ToolDescriptor {
    pub description: String,
    pub required_capabilities: BTreeSet<Capability>,
}

pub struct RegisteredToolSpec<P> {
    pub path: P,
    pub descriptor: ToolDescriptor,
}
```

因此新增 tool 的 path 只出现在注册点：

```rust
app_tool_plane.register(app_tool_path(["read"]), ReadTool);
```

具体 `Path` 需要能给 policy/audit 提供稳定显示值，但这是通过 `ActionMeta` /
plane-provided formatting 暴露，不是通过全局 `ToolPath` 类型泄漏。

## Tool Invocation Action

Tool dispatch 也是 action，但它不是 `FsReadAction` 这种 domain side-effect action。
它只授权 app orchestration 进入一个 `ToolImpl`：

```text
App execute_kernel_tool_request
  -> canonicalize request
  -> AppToolPlane.resolve(path)
  -> AppToolInvocationAction(path, required_caps, payload)
  -> Kernel::grant(action)
  -> ActionGrant<AppToolInvocationAction>
  -> AppToolPlane.invoke(Granted<AppToolInvocationAction>, &ctx)
  -> ToolImpl::execute(&ctx, input)
  -> ctx.access().fs().read_file(...)
  -> FsReadAction
  -> PolicyPipeline::grant(ctx, FsReadAction)
  -> Granted<FsReadAction>::run(ctx)
  -> filesystem side effect
```

这意味着 tool invocation policy 和 fs read policy 是两层不同授权：

- `AppToolInvocationAction`：允许调用 app plane 上某个 path 的 tool。
- `FsReadAction`：允许读取某个 canonical path。

不能用 `AuthorizedToolInvocation` 这类 receipt workaround 表达这个关系；应该返回
`ActionGrant<AppToolInvocationAction>` / `Granted<AppToolInvocationAction>`。

如果后续需要在 core 提供公共 helper，只能是泛型：

```rust
pub struct ToolInvocationAction<P> {
    path: P,
    required_capabilities: Vec<Capability>,
    payload: Value,
}
```

但当前更推荐 app/plane 定义 concrete action。这样 policy 可以通过 `ActionMeta`
观察它，kernel 可以 grant 它，core 不需要知道 path 类型。

## ToolPlane

`ToolPlane` 属于 app runtime，不属于 kernel。

目标 app-owned plane 形状：

```rust
trait ToolPlane<C: ContextFactory> {
    type Path: Clone + Ord;
    type InvocationAction: ActionMeta;

    fn resolve(&self, path: &Self::Path) -> Result<&RegisteredTool<C>, ToolPlaneError>;

    async fn invoke(
        &self,
        grant: Granted<Self::InvocationAction>,
        ctx: &C::Cx<'_>,
    ) -> Result<ToolOutcome, ToolPlaneError>;
}
```

不再使用 `ToolPayloadMatch` / `match_payload` 这种 payload-claim 机制。一个 path
命中后就由对应 tool 自己 parse 和内部流转；如果 `read` 同时支持 file/query/glob，
那它就是一个 aggregate `ReadTool`，内部解析并分流，而不是靠 plane 先看 payload 决定
是否 fallback。

`invoke` 消费 `Granted<Self::InvocationAction>`，所以 typed tool 不能绕过 policy grant
执行。`ErasedTool` 保持 private/sealed，避免 concrete tool implementer 绕过 plane 的
grant/audit wrapper。

新增 concrete tool 的目标改动面：

1. 增加一个 concrete type 并 `impl ToolImpl<C>`。
2. 在 app/bootstrap/builtin 注册点添加一条 `register(path, Tool)`。

不要为了新增工具去改 dispatcher match、catalog 拼装分支或 policy preflight 分支。

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

Kernel::record_tool_invocation(&ctx, path_display, required_caps, outcome)
```

tool invocation 的治理入口不需要知道 concrete path 类型；它只需要 `ActionMeta` 提供
operation/payload/required capabilities。pack boundary、token boundary、policy pipeline
仍在 kernel 检查。

`record_tool_invocation` 记录的是 plane 给出的 stable display path / serialized path，
不是 contracts 定义的全局 `ToolPath`。ToolImpl 本身不拿 audit capability。

旧 `Kernel::execute_tool_core` 暂时只服务 legacy fallback。旧 `CoreToolAdapter` /
`ToolExtensionAdapter` 只能被 `LegacyToolPlane` 包住，迁移完后删除。

## Audit

typed tool audit event 不记录 `ToolInvocationRoute`。route 是 app orchestration 的决策，
不是 contracts/kernel 的稳定概念。

当前事件只记录：

```rust
ToolInvocation {
    pack_id,
    path_display,
    required_capabilities,
    outcome,
}
```

这里的 `path_display` 是 audit payload，不是 registry key 类型。不同 `ToolPlane` 可以有
不同 path model，只要能在 audit 中给出稳定、可读、可关联的表示。

legacy adapter 仍暂时记录旧 `PlaneInvoked`，直到对应工具迁移完成。

## `file.read` 当前迁移状态

- 目标是 `read` 作为 aggregate typed tool 进入 app plane。
- `ReadFileTool` 只解析 payload、调用 `ctx.access().fs().read_file(...)`、格式化响应。
- 文件读取副作用发生在 `loong_access::fs`。
- `read { path }` 分支走 file read access。
- `read { query }` / `read { pattern }` / `read { glob }` 后续迁入 typed `ReadTool`
  内部分流；迁移前只能作为明确 TODO 的 legacy bridge，不作为新 plane 的 payload claim
  设计。
- `read { path, offset: 0 }` 是 typed input error，不 fallback。

## 下一步

- 删除 contracts/core 全局 `ToolPath` 设计：tool descriptor 不带 path，path 类型归
  concrete ToolPlane。
- 把已提交/未提交的 `ToolInvocationAction { path: ToolPath, ... }` 改成 app/plane
  concrete action，或泛型 `ToolInvocationAction<P>`。
- 删除 `ToolPayloadMatch` / `match_payload`，把 `read` 改成 aggregate typed tool。
- 继续把 legacy `read` 的 query/glob 搜索迁到 access-backed action。
- 将 write/edit/config.import 按同样模式迁移，迁移后删除 `FilePolicyExtension` 对应旧分支。
- 把 legacy `Kernel::execute_tool_core` 调用面逐步清空，再删除 `LegacyToolPlane` 和 adapter
  trait。
- 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration。
