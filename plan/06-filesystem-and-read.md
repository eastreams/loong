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
- `FsResolutionContext` 与 `FsPathPolicyContext` 是两个独立职责；`AccessCx::fs()` 同时约束二者，
  因为每个可执行 fs operation 都必须先完成 resolution，再把 path facts 交给 containment policy。
  同时约束不等于把两类数据重新塞回一个 aggregate context trait。
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

每个 fs operation 拥有自己的 `thiserror` error，并保留 path、policy 与 backend source。
`FsPathError` 只表达所有 path grant chain 真实共享的 prerequisite failure；不存在重新汇总所有
operation variant 的 `FsAccessError`。legacy string reason 只在旧 tool envelope 的最后边界转换，
不能反向污染 Access error。

## Operation Module Ownership

filesystem 模块按 operation 纵向切片，而不是按 action、facade method、execution 和 output
横向切片。一个 operation 的 concrete `Action`、options、`FsAccess` method、
`impl Action::run` 和 output 由同一个文件拥有：

```text
fs/
  access.rs          # FsAccess facade identity + constructor
  path.rs            # path facts + resolve/path actions + shared FsPathError
  read.rs
  write.rs
  copy.rs
  directory.rs
  inspect.rs
  glob.rs
  read_dir.rs
  content_search.rs
  remove.rs
  remove_dir.rs
  rename.rs
```

- operation modules 都保持 private；`fs` facade 显式
  re-export 每个 operation 的 public members。调用者只依赖
  `loong_access::fs::{FsAccess, FsReadAction, ...}`，不能依赖 `fs::action::*` 或 operation module
  path。
- 不建立 `fs/action/` 再按 action type 拆文件；那仍然是横向 action bucket。每个 concrete action
  移入其 operation file，并由 `loong_access::fs` 这个 owning Access facade re-export；crate root
  不再额外 flatten 这些名字。
- `access.rs` 只保留 facade identity 和构造；强制 resolve -> path grant 顺序的共享实现由
  `path.rs` 拥有，因为它只服务 path prerequisite。
- 每个 operation error 与 operation 共置，只包含该 operation 真正产生的 variant，并通过
  `FsPathError` 复用 path prerequisite failure。不得恢复 shared mega enum、泛型 error wrapper、
  分类 helper 或只转发 `From` 的兼容层。
- 每个 operation module 顶部用简短注释说明其授权与副作用边界；不写复述代码的注释。
- 当前 concrete action、Access method、`Action::run` 与 output 已按 operation 共置；内部 module
  path 保持 private，`loong_access::fs` 是唯一公共 facade。
- 纯 lexical path normalization 由 `fs::normalize_path_lexically` 统一拥有。它不访问文件系统、
  不 canonicalize symlink、也不授权路径；Session root materialization 与 action resolution 共享它，
  防止 policy 前出现两套 `.`/`..` 语义。

## Read Family

`ReadTool` 是 aggregate tool，不是 aggregate action：

- `read { path }` -> `FsReadAction`；
- `read { query }` -> `FsContentSearchAction`；
- `read { glob/pattern }` -> `FsGlobAction`。

concrete tool 只 parse payload、调用上述 Access operation、格式化 typed output。它不能直接使用
`std::fs`，不能把 input error fallback 到 legacy search，也不能恢复 direct policy preflight。

## Cancellation Boundary

- Invocation cancellation 后不再开始新的 resolve/path/operation grant。
- long-running search/glob/read-dir 应在可安全停止的迭代边界观察 cancellation。
- 已经进入不可中断 syscall 或 atomic commit 的操作不承诺回滚。generic Access execution evidence
  尚未建立；`Granted` 已固定携带同一次 mint 的 correlation metadata，但步骤 21 仍需确定 generic
  Access execution lifecycle 的唯一 owner 与 started/terminal/cancelled 状态机。不能从 authorization
  evidence 推断 operation 已完成或失败。
- cancellation signal 来自当前 Invocation Context，不成为 fs policy input，也不改变 Action required
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
