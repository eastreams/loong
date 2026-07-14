# plan: Filesystem Path Grants

`read`、`write`、`edit`、glob 和 content search 已经证明这条 fs authorization chain。这里保留
安全不变量和剩余 backend 工作，不再记录已完成的 `file.read` 迁移历史。

## Governed Path Chain

每个 filesystem operation 固定经过：

```text
raw path
  -> grant FsResolvePathAction<M>
  -> run -> ResolvedFsPath<M>
  -> grant FsPathAction<M>
  -> run -> GrantedFsPath<M>
  -> grant concrete operation Action
  -> run -> filesystem side effect
```

三个授权点语义不同：

- `FsResolvePathAction<M>` 授权 canonicalize、existing-ancestor 和 symlink observation。构造 action
  必须纯净；granted run 才观察 filesystem，并只产出不可伪造的 resolved fact。
- `FsPathAction<M>` 不再观察 filesystem。typed allowed-roots policy 根据 resolved fact 和
  `FsPathPolicyContext` 决定是否 mint path grant。
- concrete operation action 接收 path grant，声明自己的 read/write capability、payload 和
  operation policy；只有它的 granted run 才执行最终 side effect。

## Typestate

- target-following operation 使用 `TargetPath` -> `GrantedPath`。
- unlink/rename 等 final-component no-follow operation 使用 `EntryPath` -> `GrantedEntryPath`。
- 两种 marker sealed；external caller 不能实现新的 path mode，也不能从 `PathBuf` mint grant。
- `FsReadAction`、`FsWriteAction`、glob/search/read-dir/inspect/copy/create 等只接受 target grant。
- remove/rename 只接受 entry grant，避免 final symlink 被当作 target 跟随。

## Context Requirement

- `FsResolutionContext::fs_resolution_root()` 是 Access 构造 resolve action 的 execution input。
- `FsPathPolicyContext::fs_allowed_roots()` 只被 typed path policy 读取。
- `AccessCx::fs()` 只要求 resolution view；`FsPathAllowedRootsPolicy` 单独约束 path policy view。
- capability 由通用 `PolicyContext` 提供；不再存在把 resolution、allowed roots 和 capabilities
  混在一起的 `FsAccessPolicyContext`。

## FsAccess

`FsAccess` 可以保留 private `grant_target_path`、`grant_entry_path` 和共享 `grant_path`。它们存在
的唯一理由是强制所有 fs operation 按相同顺序消费 resolve grant 和 path grant；不能做 policy
decision、fallback 或最终 side effect。

普通 caller 只看到：

```rust
ctx.access().fs().read_file(path).await
ctx.access().fs().write_file(path, bytes, options).await
```

每个 fs domain error 使用 `thiserror` 保留 source。legacy string reason 只在旧 tool envelope 的
最后边界转换，不能反向污染 Access error。

## Read Family

`ReadTool` 是 aggregate tool，不是 aggregate action：

- `read { path }` -> `FsReadAction`；
- `read { query }` -> `FsContentSearchAction`；
- `read { glob/pattern }` -> `FsGlobAction`。

concrete tool 只 parse payload、调用上述 Access operation、格式化 typed output。它不能直接使用
`std::fs`，不能把 input error fallback 到 legacy search，也不能恢复 direct policy preflight。

## Cancellation Boundary

- Turn cancellation 后不再开始新的 resolve/path/operation grant。
- long-running search/glob/read-dir 应在可安全停止的迭代边界观察 cancellation。
- 已经进入不可中断 syscall 或 atomic commit 的操作不承诺回滚。generic Access execution evidence
  尚未建立；它必须等步骤 20 确定唯一 consumption owner 与 correlation carrier 后再实现，不能从
  authorization evidence 推断 operation 已完成或失败。
- cancellation signal 来自当前 Turn Context，不成为 fs policy input，也不改变 Action required
  capabilities。

## 剩余安全缺口：TOCTOU

`GrantedPath` / `GrantedEntryPath` 目前证明 policy 接受了 resolution 时观察到的 pathname，但不
pin inode、directory descriptor 或 object handle。另一个 actor 仍可能在 authorization 与最终
operation 之间替换 pathname。

目标 backend：

- 优先采用成熟 descriptor-relative/capability-based filesystem library，不手写未经审计的 syscall
  wrapper。
- authorization 和最终 side effect 消费同一受约束 handle/descriptor chain，不重新按已授权
  `PathBuf` 解析。
- target-following 与 final-component no-follow 继续由类型区分。
- Unix/Windows 保证不一致时，在 backend contract 和 platform tests 显式表达，不能静默降级。
