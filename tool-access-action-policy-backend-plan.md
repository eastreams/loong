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
- `loong-tools`：concrete builtin tool implementations only。这个 crate 不承载
  `ToolImpl`、`RegisteredTool`、registry、plane、policy/action 抽象；它只放
  `ReadFileTool` 这类具体工具和它们的 payload/response helper。

## 基础小改动优先队列

这些改动不改变架构终局，但会减少后续大迁移的噪音。后续实现应优先以最小提交完成
这些地基项，再做 `ToolPath` / aggregate `ReadTool` / legacy adapter 删除。

1. Cargo workspace dependency hygiene：
   - 新增或迁移内部 Loong crate 依赖时，先在根 `Cargo.toml` 的
     `[workspace.dependencies]` 加一条统一声明；
   - 叶子 crate 使用 `loong-*.workspace = true`，不要重复写
     `package/version/path`；
   - 已完成当前 access/action/tool 链：
     `loong-access`、`loong-contracts`、`loong-core`、`loong-kernel`、
     `loong-plugin-sdk`、`loong-tools`；
   - 历史 daemon/spec/bridge 依赖不要混进功能提交里大扫除，除非该提交正好触碰这些
     crate。

2. Concrete builtin tools crate hygiene：
   - `crates/tools` 只放具体 builtin tools；
   - 不在 `crates/tools` 定义或 re-export `ToolPlane`、`ToolImpl`、`RegisteredTool`、
     `Policy`、`AccessCx` 这类抽象；
   - app feature 只负责开关 concrete tool feature，例如
     `tool-file = ["loong-tools/file"]`；
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
     `ToolPlane::invoke(Granted<AppToolInvocationAction>, &ctx)`。

5. `ActionMeta::payload` borrowing hygiene：
   - 当前 `ActionMeta::payload(&self) -> Value` 仍会强迫持有 JSON payload 的 action
     clone；
   - 目标签名是 `fn payload(&self) -> Cow<'_, Value>`；
   - 不提供默认 `Null`，每个 action 都必须显式声明自己的 type-erased payload；
   - 对天然可借用的 action，返回 `Cow::Borrowed(&self.payload)`；
   - 对需要临时构造 JSON view 的 action，返回 `Cow::Owned(json!(...))`；
   - 这是小基础改动，应在搬 `ToolInvocationAction` 或拆 `ToolSpec.path` 之前完成，避免后续
     action 迁移继续复制旧签名。

6. Comment audit hygiene：
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
     - `crates/app/src/tools/plane.rs`：注释必须讲清 app owns typed plane，kernel 不持有
       typed registry；plane 按 path resolve，不按 payload claim；`invoke` 消费
       `Granted<AppToolInvocationAction>`，所以 concrete tool implementer 不需要也不能自己做
       audit/grant。
     - `crates/app/src/tools/mod.rs`：typed dispatch 边界附近要讲清它只是迁移期
       orchestration：resolve -> invocation action -> kernel grant -> plane invoke ->
       kernel audit；`read` query/glob legacy bridge 是临时桥，不是 payload-claim 设计。
     - `crates/kernel/src/kernel.rs`：`grant_tool_invocation` / `record_tool_invocation`
       注释必须讲清 kernel 是 governance authority，不执行 typed tool；tool invocation
       grant 只授权进入 `ToolImpl`，tool 内部 side effect 仍需自己的 access action grant。
     - `crates/contracts/src/audit_types.rs`：`ToolInvocation` / `ToolInvocationOutcome`
       注释必须讲清 audit 记录的是一次 tool invocation attempt 的结果；它不表达
       `ToolInvocationRoute`，也不能固化 concrete ToolPlane registry key 类型。
     - `crates/loong-core/src/policy/action.rs`：`ActionMeta::payload()` 注释必须讲清 payload
       是 Action 的 type-erased structured view，不是 legacy bridge；目标签名是
       `Cow<'_, Value>`，并且没有默认 `Null`。
     - `crates/loong-core/src/policy/grant.rs`：`Granted<A>::as_ref()` 注释必须讲清它只用于
       grant 被消费前的 audit/metadata inspection，不能成为伪造、复制或绕过执行边界的入口。
     - `crates/app/src/tools/routing.rs`：legacy read bridge 注释必须讲清 query/glob 暂未迁入
       aggregate `ReadTool`；迁移完成后删除 bridge，而不是把它提升成长期 routing 机制。
   - 注释验收标准：读者只看相关类型/函数附近的注释，就能回答“这个层拥有谁”“为什么不在
     另一个 crate”“这个 fallback 是否长期存在”“谁可以做副作用”“grant 何时被消费”；
   - typed path 测试只断言 typed audit，legacy path 测试只断言 legacy audit；
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

typed tool invocation 应该有 audit event。tool 调用是 agent/user 可见的治理边界；
成功、失败、拒绝都必须能在 audit 里查到。否则 typed tool 从 legacy
`PlaneInvoked` 迁走后，证据链反而变少。

但 audit event 不能固化 ToolPlane 的 registry key 类型。当前把
`ToolInvocation { path: ToolPath, ... }` 加到 `AuditEventKind` 里是过度固化：audit
需要的是可读、稳定、可关联的 path 表示，不是具体 plane 的 key。`ToolPath` 不应该成为
contracts/core 的全局类型。

typed tool audit event 不记录 `ToolInvocationRoute`。route 是 app orchestration 的决策，
不是 contracts/kernel 的稳定概念，也不该成为长期 contract。

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

`ToolInvocationOutcome` 放在 contracts 可以接受，因为它是 audit payload 的稳定结果形状。
但它只能描述一次 tool invocation attempt 的结果，不能隐含 app plane route、fallback
机制或 concrete registry 实现。

legacy adapter 仍暂时记录旧 `PlaneInvoked`，直到对应工具迁移完成。

## 当前实现偏差

以下是已经出现、但不应该继续放大的过渡形状：

- `crates/contracts/src/audit_types.rs` 里 `AuditEventKind::ToolInvocation` 持有
  `ToolPath`。这把 concrete plane path 细节泄漏到了 contracts。
- `crates/contracts/src/tool_types.rs` 仍定义全局 `ToolPath`，且 `ToolSpec` 仍携带
  `path`。目标是 descriptor 无 path，注册点/plane 才绑定 path。
- `crates/loong-core/src/tool.rs` 里的 `ToolInvocationAction` 持有全局 `ToolPath`。
  这属于 app/plane-owned action，或者至少应该是 `ToolInvocationAction<P>`。
- `crates/tools/src/file.rs` 的 `ReadFileTool::spec()` 仍返回带 path 的 `ToolSpec`。
  这是 tool descriptor 与 registration path 未拆开的直接症状。
- `crates/app/src/tools/mod.rs` 里 typed dispatch、grant、invoke、audit 逻辑还堆在
  `execute_kernel_tool_request`。目标是 app orchestration 拥有这段边界，但函数应更聚焦。
- `crates/app/src/tools/routing.rs` 的 `route_direct_read_tool_request_for_legacy` 名字不准。
  它现在同时承担 read surface normalization 和 legacy bridge；aggregate `ReadTool`
  落地后这块应该删除或拆清楚。
- 若测试用 `PlaneInvoked | ToolInvocation` 同时接受，就会掩盖 typed/legacy route 回退。
  typed path 应断言 typed audit，legacy path 应断言 legacy audit。

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

按最小提交顺序推进：

0. 先完成基础小改动队列：
   - 对本轮要碰的 crate 先做 workspace dependency hygiene；
   - 把 `ActionMeta::payload` 改成 `Cow<'_, Value>`；
   - 执行 comment audit checklist，补齐 concrete tools crate、`KernelAccess`、grant
     inspection、typed plane、kernel grant/audit、legacy read bridge 等边界注释；
   - 清掉会掩盖 route 回退的宽松测试断言；
   - 每个小项独立提交，不和下面的大结构迁移混在一起。

1. 修正 `ToolInvocation` audit shape：
   - 保留 `AuditEventKind::ToolInvocation`；
   - 把 `path: ToolPath` 改成 `path_display: String` 或等价 audit-only 表示；
   - 保留 `ToolInvocationOutcome`，但注释说明它只描述 invocation attempt 结果；
   - 更新 kernel/app 测试，typed path 不再依赖全局 `ToolPath`。

2. 移走或泛型化 `ToolInvocationAction`：
   - 首选把它移到 app plane 附近，命名为 `AppToolInvocationAction`；
   - 如果保留公共 helper，则改成 `ToolInvocationAction<P>`；
   - kernel grant API 接受 concrete action，不知道 app plane path type；
   - 删除 `loong-core` 对 `ToolPath` 的依赖。

3. 清理 tool descriptor/path 耦合：
   - `ToolImpl::spec()` 返回无 path descriptor；
   - `RegisteredTool` 只保存 descriptor/provenance/registration metadata；
   - `AppToolPlane::register(path, tool)` 组合 path + descriptor；
   - `ReadFileTool::spec()` 不再硬编码 `"read"`。

4. 收敛 app typed dispatch 边界：
   - 从 `execute_kernel_tool_request` 中抽出一个聚焦的 app orchestration 边界；
   - 该边界只做 resolve -> build invocation action -> kernel grant -> plane invoke ->
     kernel audit；
   - 不引入 `AuthorizedToolInvocation` receipt workaround。

5. 改 `read` 为 aggregate typed tool：
   - 删除 payload-claim/fallback 思路；
   - `ReadTool` 内部解析 `path/query/pattern/glob`；
   - `read { path, offset: 0 }` 是 typed input error，不 fallback；
   - `read { query }` / `read { pattern }` / `read { glob }` 迁入 typed path 后，旧
     direct read legacy bridge 删除。

6. 继续迁移剩余 legacy side-effect tools：
   - write/edit/config.import 按同样 access-backed action 模式迁移；
   - 迁移完成后删除 `FilePolicyExtension` 对应旧分支；
   - 逐步清空 `Kernel::execute_tool_core` 调用面，再删除 `LegacyToolPlane` 和 adapter
     trait。

7. 测试清理：
   - typed tool 测试只接受 `ToolInvocation` audit；
   - legacy adapter 测试只接受 `PlaneInvoked` audit；
   - 不用 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 每个最小提交跑对应 targeted tests、`cargo check` 和 `git diff --check`。

8. 将 config-driven policies 全部迁入 app bootstrap 的 typed policy registration。
