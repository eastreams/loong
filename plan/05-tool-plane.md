# plan: ToolPlane 与 Tool Invocation

本文件定义 ToolPlane、plane-local path、tool invocation action 和 registry 形状。它不定义
具体 builtin tool 的实现细节。

## Tool Path

`path` 概念保留，但 path 类型归属从 contracts/core 移到具体 plane。

禁止形状：

```rust
// contracts/core globally decide every plane's path model.
pub struct ToolPath(String);

pub struct ToolSpec {
    pub path: ToolPath,
    // ...
}
```

目标形状示意。示例中的 `ToolPath` 是 plane-local 类型，不是 contracts/core 全局类型：

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

示例中的 `ToolPath` 是 `loong-app::tools::plane` 内的 plane-local path，可以是
`Vec<String>`、smallvec、interned path、trie key，或后续其它形状。层级由模块路径表达，
不靠类型名前缀表达；也不是 contracts/core 的决定。

`ToolSlot` 只是 `ToolPlane` 内部注册句柄。外部调用、audit payload、kernel grant、
contracts/core 都不暴露 slot；它们只看 plane 提供的 path display / action payload。
`paths` 负责把 registry path resolve 到 slot，`entries` 承载 `RegisteredTool` 本体、
provenance 和注册元数据。未来如果 path index 换成 trie，只替换 `paths` 这一层，
不用改 entry storage 或 concrete tool。
`ToolEntry` 不保存 path，避免和 `paths` index 形成可 drift 的重复状态。本迁移阶段不设计
slot-based unregister；如果后续需要，再补 reverse index 或明确 owner invariant。

tool 自身返回无 path descriptor：

```rust
pub struct ToolDescriptor {
    pub description: String,
    pub required_capabilities: BTreeSet<Capability>,
    pub argument_hint: Option<String>,
    pub search_hint: Option<String>,
    pub tags: Vec<String>,
}
```

如果 catalog / agent prompt 需要“path + descriptor”的视图，由 plane 在列举时从
`paths` index 和 entry descriptor 按需投影出来；不要先固定一个 core-level
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
  -> ctx.tool(path)?
  -> ToolInvocation::invoke(payload)
  -> compute child caps and invocation overlay
  -> loong-app::tools::plane::ToolInvocationAction(path, required_caps, payload)
  -> Kernel::grant(action)
  -> Granted<ToolInvocationAction>
  -> ToolPlane.invoke(Granted<ToolInvocationAction>, &child_ctx)
  -> ToolImpl::execute(&child_ctx, input)
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

不能用 `AuthorizedToolInvocation` 这样的 receipt workaround 表达 tool invocation grant
与 concrete tool dispatch 的关系；应该返回
`ActionGrant<ToolInvocationAction>` / `Granted<ToolInvocationAction>`。

`ToolInvocationAction` 在本迁移阶段不放进 core。它放在
`crates/app/src/tools/plane.rs` 附近，使用 app plane 自己的 path 类型和 stable display。
如果后续需要在 core 提供公共 helper，只能是泛型：

```rust
pub struct ToolInvocationAction<P> {
    path: P,
    required_capabilities: Vec<Capability>,
    payload: Value,
}
```

Core generic helper 不属于该步骤。该步骤的目标是 app/plane 定义 concrete action。这样 policy 可以
通过 `ActionMeta` 观察它，kernel 可以 grant 它，core 不需要知道 path 类型。


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
plane 的 grant wrapper。`ToolPlane::invoke` 是 granted primitive；普通调用点应使用
`ctx.tool(path)?.invoke(payload).await`，让 app context 派生的 invocation handle 负责
build action、kernel grant 和 audit。

自动 grant 的 shortcut 不放在 `ToolPlane` trait 上。它挂在 app-defined context 派生出的
`ToolInvocation<'_>` handle 上：`ctx.tool(path)` 先做 plane-local path 解析/entry lookup，
因此返回 `Result<ToolInvocation<'_>, ToolLookupError>`；`invoke(payload)` 才读取 descriptor、
计算 child caps、构造 invocation action、调用 kernel grant、再把 grant 交给 plane。
`ToolPlane` trait 只表达“已授权 invocation 如何 dispatch”，不知道 kernel、token、pack、
audit sink 或 event id。这样 concrete tool 可以通过 ctx 做受治理的 tool->tool 调用，但
仍然拿不到裸 audit API。

`ToolInvocation<'_>` 是调用 handle，不是 authorization receipt。它可以携带 optional caps
override 和 trusted overlay，但不能预先持有 grant；grant 必须在拿到 payload 后构造
`ToolInvocationAction` 时发生，因为 policy 可能需要观察 action payload。

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
