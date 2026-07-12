# plan: Filesystem Path Grants 与 `read`

本文件定义 filesystem path authorization 和 `read` 迁移状态。它只讨论 fs/read domain，
不重复 tool plane 或 kernel audit 的通用规则。

## Filesystem Path Grants

“得到一个可供 fs action 使用的路径”是独立 action，不是 read action 的构造细节。
目标调用形状分三步：

1. `FsAccess` 使用 access/fs 定义、kernel 导出的 `FsResolutionContext::fs_resolution_root()`
   准备 resolved path facts。这一步需要 canonicalize、existing ancestor resolution、
   symlink resolution，因此属于 access 边界的 filesystem observation。
2. `PolicyPipeline` 对 `FsResolvePathAction` 做 typed policy 决策。allowed roots 来自
   `FsPathPolicyContext::fs_allowed_roots()`，是 policy input，不塞进 action；
   path escape 由 kernel policy deny，denial 进入 `PolicyReport`。
3. 只有 granted resolve action 的 `run` 能 mint `GrantedPath`。下游 read/write/copy/create-dir/
   search/glob/inspect action 只能接收 `GrantedPath`，不能接收 raw path 或普通 `PathBuf`。

核心类型：

```rust
pub struct FsResolvePathAction {
    raw_path: PathBuf,
    resolved_path: PathBuf,
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
)?;
let grant = policy_engine.grant(ctx, resolve).await?;
let path = grant.granted.run(ctx).await?;

let read = FsReadAction::new(path);
let grant = policy_engine.grant(ctx, read).await?;
grant.granted.run(ctx).await
```

这条链路有两个不同授权点：

- `FsResolvePathAction`：允许在本次 `Context` 下把 raw path 解析成 `GrantedPath`。
  action 携带 access 准备好的 resolved path facts；policy 只基于这些 facts 表达
  workspace root、file root、path escape、symlink escape 等路径权限，并从 context view
  读取 allowed roots。
- `FsReadAction` / `FsWriteAction` / `FsCopyFileAction` / `FsCreateDirAllAction` /
  `FsContentSearchAction` / `FsGlobAction` / `FsInspectPathAction`：允许对一个已经治理过的
  `GrantedPath` 执行具体读写、文件复制、目录创建、内容搜索、路径枚举或路径元数据观察。
  它们仍然各自声明 capability 和 payload，因为副作用和泄漏面不同。

`FsResolvePathAction::run` 不再重新 canonicalize，也不读取文件内容。它只消费
`Granted<FsResolvePathAction>` 并把 policy 已接受的 resolved facts 变成 `GrantedPath`。
如果解析结果逃逸 allowed roots，kernel typed policy 会 deny，因而不会产出
`GrantedPath`。

workspace guidance / runtime-self source discovery 也遵守同一边界：候选根只做
deterministic discovery，不能借 `Path::is_dir` 跟随 nested workspace symlink 到
workspace 外部。nested root 必须 canonicalize 后仍位于 canonical workspace root 内；真正
读取文件内容仍只能通过 governed fs access。

不要写 `FsReadAction::new(Granted<FsResolvePathAction>, ctx)` 这种隐藏执行的 API；
先显式 `grant.granted.run(ctx).await?`，再把 `GrantedPath` 交给下游 action。

注册规则：

- `PolicyPipeline::new()` 是 default deny；没有 terminal allow policy 的 action 会被拒绝。
- legacy fallback 只能通过 `PolicyPipeline::new_legacy_allow_fallback()` 显式选择，而且只服务
  legacy kernel action，不能 grant typed access/tool action。
- 任何会执行 `file.read` / `read { path }` 的 app runtime、test harness、helper
  都必须显式注册 `FsResolvePathAction` 的 allowed-roots policy 和 `FsReadAction`
  的 terminal allow policy。否则 typed read 应该 fail closed，而不是被 fallback 放过。
- `FsCopyFileAction` / `FsCreateDirAllAction` / `FsInspectPathAction` / `FsGlobAction` /
  `FsContentSearchAction` 的 terminal allow policy 已经在 app/bootstrap 注册。
  kernel/context-aware `read { pattern/glob/query }` 会走 typed tool/policy path；
  无 context 的 legacy read 入口 fail closed，不能执行 read side effect。

## `file.read` 迁移状态

- 目标是 `read` 作为 aggregate typed tool 进入 app plane，但它内部分出来的 action
  不能聚合。
- `ReadTool` 是 app plane 注册的 aggregate typed tool。它只解析 payload、调用
  `ctx.access().fs()` 下的具体 operation、格式化响应。
- 文件读取副作用发生在 `loong_access::fs`。
- `read { path }` 分支走 `FsResolvePathAction -> GrantedPath -> FsReadAction`。
- `read { pattern }` / `read { glob }` 走 `FsGlobAction`。
- `read { query }` 走 `FsContentSearchAction`。
- kernel/context-aware direct read 已经通过 `ctx.tool("read")?.invoke(...)` 进入 typed
  `ReadTool`；这条路径不再把 query/glob fallback 到 legacy `content.search` /
  `glob.search`。
- 无 context 的 legacy `execute_tool_core_with_config(read)` 会 fail closed，要求 kernel
  access context；不能恢复成 query/glob legacy side effect。
- concrete `ReadTool` 不能直接 `std::fs::read_dir` / `std::fs::read` 临时补 side effect。
- `read { path, offset: 0 }` 是 typed input error，不 fallback。
