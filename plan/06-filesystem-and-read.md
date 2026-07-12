# plan: Filesystem Path Grants 与 `read`

本文件定义 filesystem path authorization 和 `read` 迁移状态。它只讨论 fs/read domain，
不重复 tool plane 或 kernel audit 的通用规则。

## Filesystem Path Grants

“得到一个可供 fs action 使用的路径”是独立 action，不是 read action 的构造细节。
目标调用形状分四步：

1. `FsAccess` 从 `FsResolutionContext::fs_resolution_root()` 构造纯数据的
   `FsResolvePathAction`。构造 action 不观察文件系统；只有 policy grant 后的 `run` 才执行
   canonicalize、existing ancestor resolution 和 symlink resolution，并产出不可由调用方伪造的
   `ResolvedPath` / `ResolvedEntryPath`。这两个值只表示解析事实，不表示路径已被允许。
2. `FsPathAction` 接受 resolved fact，不再执行任何 filesystem observation。
   `PolicyPipeline` 对它做 allowed-roots typed policy 决策。allowed roots 来自
   `FsPathPolicyContext::fs_allowed_roots()`，是 policy input，不塞进 action；
   path escape 由 kernel policy deny，denial 进入 `PolicyReport`。
3. 只有 granted `FsPathAction` 的 `run` 能 mint path grant。target-following 路径变成
   `GrantedPath`，final-component no-follow 路径变成 `GrantedEntryPath`。下游具体 action
   只能接收与自身操作语义一致的 grant type，不能接收 raw path 或普通 `PathBuf`。
4. 具体 fs action 接收 granted path，声明自身 read/write capability 和 operation policy；只有它的
   grant 被消费后才能执行最终 filesystem side effect。

核心类型：

```rust
pub struct FsResolvePathAction<M> {
    raw_path: PathBuf,
    resolution_root: PathBuf,
}

pub struct ResolvedFsPath<M> {
    requested: PathBuf,
    resolved: PathBuf,
}

pub struct FsPathAction<M> {
    resolved: ResolvedFsPath<M>,
}

pub struct GrantedFsPath<M> {
    path: PathBuf,
}

impl<M> GrantedFsPath<M> {
    pub fn as_path(&self) -> &Path;
    pub fn into_path_buf(self) -> PathBuf;
    // no public from/pathbuf constructor
}
```

调用链：

```rust
let resolve_action = FsResolvePathAction::target(
    raw_path,
    ctx.fs_resolution_root(),
);
let resolved = policy_engine
    .grant(ctx, resolve_action)
    .await?
    .granted
    .run(ctx)
    .await?;
let path = policy_engine
    .grant(ctx, FsPathAction::new(resolved))
    .await?
    .granted
    .run(ctx)
    .await?;

let read = FsReadAction::new(path);
let grant = policy_engine.grant(ctx, read).await?;
grant.granted.run(ctx).await
```

这条链路有三个不同授权点：

- `FsResolvePathAction`：允许执行路径解析 observation。它的 typed allow policy 只允许生成
  opaque resolved fact，不会授予后续 filesystem operation authority；构造 action 本身必须纯净。
- `FsPathAction`：允许把 resolved fact 变成受治理的 path grant。policy 只基于 resolved fact 表达
  workspace root、file root、path escape、symlink escape 等路径权限，并从 context view
  读取 allowed roots。target 与 entry 使用同一个 generic `FsPathAllowedRootsPolicy` 实现，
  但分别注册 concrete typestate，避免每个 fs operation 重复 containment policy，也避免把
  no-follow entry grant 误交给 target-following read。
- `FsReadAction` / `FsWriteAction` / `FsCopyFileAction` / `FsCreateDirAllAction` /
  `FsContentSearchAction` / `FsGlobAction` / `FsReadDirAction` /
  `FsInspectPathAction`：允许对一个已经治理过的 `GrantedPath` 执行具体读写、文件复制、
  目录创建、内容搜索、递归路径匹配、一层目录枚举或路径元数据观察。
  它们仍然各自声明 capability 和 payload，因为副作用和泄漏面不同。
- `FsRemoveFileAction` / `FsRemoveDirAllAction` / `FsRenameAction`：只接收
  `GrantedEntryPath`。ancestor symlink 会在 granted `FsResolvePathAction::entry` 运行时解析给
  path policy 检查，final symlink 不跟随，因此 remove/rename 操作的是 entry 本身而不是 target。

`FsPathAction::run` 不再重新 canonicalize，也不读取文件内容。它只消费
`Granted<FsPathAction<M>>` 并把 policy 已接受的 resolved fact 变成对应 typestate 的
`GrantedFsPath<M>`。
如果解析结果逃逸 allowed roots，kernel typed policy 会 deny，因而不会产出
`GrantedPath`。

`FsAccess` 允许保留 `grant_target_path`、`grant_entry_path` 和它们共享的 `grant_path` 私有
shortcut。它们存在的唯一理由是让每个具体 fs operation 都强制经过 resolve grant 和 path grant；
它们只能按固定顺序请求并消费这两个 grant，不能自行作出 policy decision，也不能加入
fallback 或最终 side effect。普通调用方仍只看到
`ctx.access().fs().read_file(path)` 这类 operation API。

`GrantedPath` / `GrantedEntryPath` 证明 policy 接受了 resolution 时观察到的 pathname，
但目前不会 pin inode 或 directory descriptor。因此该 typestate 防止代码层绕过 grant，却不能
消除另一个 actor 在 authorization 与最终 operation 之间替换路径的 TOCTOU。后续 fs backend
需要用 descriptor-relative/openat-style primitive 收紧这一执行边界；在此之前不要把 path grant
描述成稳定的 object capability。

workspace guidance / runtime-self source discovery 也遵守同一边界：候选根只做
deterministic discovery，不能借 `Path::is_dir` 跟随 nested workspace symlink 到
workspace 外部。nested root 必须 canonicalize 后仍位于 canonical workspace root 内；真正
读取文件内容仍只能通过 governed fs access。

不要写 `FsReadAction::new(Granted<FsPathAction>, ctx)` 这种隐藏执行的 API；
先显式 `grant.granted.run(ctx).await?`，再把 `GrantedPath` 交给下游 action。

注册规则：

- `PolicyPipeline::new()` 是 default deny；没有 terminal allow policy 的 action 会被拒绝。
- legacy fallback 只能通过 `PolicyPipeline::new_legacy_allow_fallback()` 显式选择，而且只服务
  legacy kernel action，不能 grant typed access/tool action。
- 任何会执行 `file.read` / `read { path }` 的 app runtime、test harness、helper
  都必须显式注册 target `FsResolvePathAllowPolicy`、target `FsPathAllowedRootsPolicy` 和
  `FsReadAction`
  的 terminal allow policy。否则 typed read 应该 fail closed，而不是被 fallback 放过。
- `FsCopyFileAction` / `FsCreateDirAllAction` / `FsRemoveFileAction` /
  `FsInspectPathAction` / `FsGlobAction` / `FsReadDirAction` / `FsContentSearchAction`
  的 terminal allow policy 已经在 app/bootstrap 注册。
  kernel/context-aware `read { pattern/glob/query }` 会走 typed tool/policy path；
  无 context 的 legacy read 入口 fail closed，不能执行 read side effect。

## `file.read` 迁移状态

- 目标是 `read` 作为 aggregate typed tool 进入 app plane，但它内部分出来的 action
  不能聚合。
- `ReadTool` 是 app plane 注册的 aggregate typed tool。它只解析 payload、调用
  `ctx.access().fs()` 下的具体 operation、格式化响应。
- 文件读取副作用发生在 `loong_access::fs`。
- `read { path }` 分支走
  `FsResolvePathAction -> ResolvedPath -> FsPathAction -> GrantedPath -> FsReadAction`。
- `read { pattern }` / `read { glob }` 走 `FsGlobAction`。
- `read { query }` 走 `FsContentSearchAction`。
- kernel/context-aware direct read 已经通过 `ctx.tool("read")?.invoke(...)` 进入 typed
  `ReadTool`；这条路径不再把 query/glob fallback 到 legacy `content.search` /
  `glob.search`。
- 无 context 的 legacy `execute_tool_core_with_config(read)` 会 fail closed，要求 kernel
  access context；不能恢复成 query/glob legacy side effect。
- concrete `ReadTool` 不能直接 `std::fs::read_dir` / `std::fs::read` 临时补 side effect。
- `read { path, offset: 0 }` 是 typed input error，不 fallback。
