# plan: Runtime / Session / Context / Crate 收敛

本文件记录 Runtime、Session、Turn Context 的剩余迁移边界，以及 transitional crate 清理。
具体提交顺序见 `08-next-steps.md`。

## 当前事实

- `loong-runtime::Runtime<C>` 已经持有 `Kernel<C>` 和 erased typed `ToolPlane<C>`。
- `loong-runtime::tool_plane` 已经拥有 plane-local `ToolPath`、`ToolInvocationAction`、
  `ToolPlane` trait 和 slot-backed `ToolPlaneRegistry`。
- app bootstrap 已使用 fallible `builtin_tool_plane()` 构造 registry；不存在需要迁移的全局
  `OnceLock` tool plane。
- app 仍以 `Arc<AppContextInner>` 表达 runtime authority、session state 和 invocation overlay。
  `AppContextFactory::Cx<'a> = AppContext` 没有使用 GAT lifetime，仍是 owned clone 模型。
- 截至 2026-07-13，`AppContext` 在 97 个 Rust 文件中出现 559 次，
  `AppContextFactory` 出现 78 次；这是 workspace-wide replacement，不是局部 rename。
- channel/conversation 仍大量传播 `ConversationRuntimeBinding`，provider 仍传播
  `ProviderRuntimeBinding`。这些 enum 把“advisory 权限”错误表达成“可能没有 Context”。
- `loong-runtime` crate root 仍保留 `RuntimeSpine`、one-shot/interactive transitional API，并
  re-export `loong_core::Session`；新的 `runtime` / `tool_plane` owner 与旧 spine 尚未收敛。

## 目标 Ownership

### Runtime

`Runtime<C>` 是一个治理域的长期 owner：

- 持有 kernel、typed tool plane 和真正属于 runtime 的长期 registry/configuration；
- 不持有 invocation Context；
- 不把 app Session/tool namespace 塞进 kernel；
- builtin/plugin registration 在 bootstrap 完成，错误通过 `Result` 返回，不在首次调用时 panic。

### Session

Session 是跨 Turn 的长期主体：

- 拥有 identity、基础 capability/token evidence、session mode 和 lifecycle state；
- 保存可被后续 Turn 继承的稳定配置/authority，不保存本次 invocation Context；
- Rust lifetime 只约束 Context 借用，不负责注销、取消、恢复或持久化 Session；
- durable Session record 与进程内活跃 owner 必须保持语义可区分，不能因为名字相同就假定是
  同一个 object lifetime。

### Context

`Context<'a>` 是一次 Turn 的执行快照：

```text
Session authority
  + typed Turn options (mode / goal / narrowing / request options)
  + Turn cancellation signal
  -> validate and normalize
  -> Context<'a>
```

Context 只包含本次执行真正需要的投影：

- 对 Runtime/Session 稳定数据的借用；
- 归一化后的 mode/goal/options；
- effective capabilities、tool config 和 fs resolution/policy views；
- 本次 Turn 的 cooperative cancellation signal；
- 其它已经证明是本次执行属性的窄数据。

Context 不包含：

- tool/action payload；
- `ExecutionPlane` / `PlaneTier`；
- mailbox、task supervisor、session registry 或持久化 repository；
- 独立 kernel/policy/audit owner；
- 仅为了满足 `'static` 而复制的 `Arc<AppContextInner>`。

构造规则：

- Context 构造是 authority normalization boundary。requested caps、tool view、roots 和其它
  override 在这里与 Session baseline 求交/校验，不能在 tool helper 中临时拼装。
- base Context 的 mode/goal/options 在执行期间不可变。nested tool invocation 派生 child
  Context 时只允许缩窄 effective caps/tool/root view，并继承 Turn cancellation。
- `Context::access()` 是 `AccessCx::new(...)` 的唯一 app concrete 构造点；普通调用点使用
  `ctx.access()`。
- `ctx.tool(path)` 只做 lookup 并返回借用型 invocation handle；`invoke(payload)` 才构造 child
  Context 和 invocation action。
- `RuntimeContextFactory` 只有 GAT：

```rust
impl ContextFactory for RuntimeContextFactory {
    type Cx<'a> = Context<'a>;
}
```

它不提供 factory method；value construction 属于 Session/turn orchestration。

## Context 破坏性替换

替换不能机械保留旧字段：

1. 将 `AppContextInner` 字段按 Runtime、Session、Turn option、Context derived view、Action
   payload 五类重新归属。
2. 删除没有真实 consumer 的 `plane` / `tier`；legacy audit route 需要时显式传入 audit
   boundary，不能借 Context 偷渡。
3. 删除 `request_parameters` 和 `KernelInvocationContext::request_parameters()`；PolicyAny 通过
   `ActionMeta::payload()` 观察当前 action，不从 Context 读取另一份请求 JSON。
4. 删除 `Deref` / `DerefMut` / `Arc::make_mut`、`child`、`for_session`、`for_invocation` 等旧 COW
   mutation API。新 Context 构造/派生必须显式表达 Turn options 或 authority narrowing。
5. advisory Session 仍构造同一种 Context，只是 Session baseline authority 没有 `InvokeTool`
   或 mutation caps。删除 `Option<AppContext>`、`ConversationRuntimeBinding::AdvisoryOnly` 和
   provider no-context 对应物，不能用“没有 Context”表达权限模式。
6. 一次性迁移 production、tests、fixtures、trait impl、generic instantiation、注释和文档中的
   concrete 名称，然后删除三个 `AppContext*` 类型；不留 compatibility alias。

## Streaming Execution

当前 `/v1/chat/completions` streaming 把 turn 放进 detached `tokio::spawn`。SSE receiver drop
只让 `send` 失败，provider/tool execution 继续运行。目标行为：

- downstream disconnect 触发当前 Context 的 cancellation；
- provider stream/read retry、tool scheduling 和长时 access operation 协作退出；
- runner 保留不可取消的 finalization boundary，记录 cancelled outcome、partial-output policy 和
  Session lifecycle transition；
- grace period 后才允许强制 drop/abort，并记录 cancellation timeout；
- 不需要为了这个目标引入永久 `SessionTask` actor。只有确实需要 per-session mailbox
  serialization/passivation 时，才单独设计 actor runtime。

## Crate 收敛

保留：

- `loong-kernel`：governance authority。
- `loong-access`：side-effect physical boundary。
- `loong-tools`：concrete builtin implementations。
- `loong-runtime`：已经拥有 `Runtime<C>` 和 ToolPlane primitive；删除旧 spine，而不是放弃
  runtime owner。
- `loong-contracts` / `loong-core`：继续按稳定 data 与 behavior contract 分工。

剩余收敛候选：

1. 删除 `loong-runtime` crate root 的 `RuntimeSpine`、one-shot/interactive transitional API 和
   仅为 phase spine 存在的 re-export；保留 `runtime` / `tool_plane` owner。
2. 审计 `loong-cli` 与 `loong-app-protocol`。如果只是 transitional CLI/protocol forwarding
   shell，合并到 daemon 或真实 protocol owner。
3. 修正 kernel -> `loong-plugin-sdk` 的反向依赖。kernel 所需 contract 下沉到 leaf；SDK 只保留
   plugin author-facing API。
4. 审计 `protocol` / `bridge-runtime` 是否拥有稳定 wire/bridge primitive；只转发的壳合并，
   真实协议边界保留。

workspace 目前有 15 个 crate，而 `AGENTS.md`、`CLAUDE.md` 和部分 architecture docs 仍写 13。
crate 事实修正必须从 `Cargo.toml` / `cargo metadata --no-deps` 重建真实 DAG，并同步镜像文档、
architecture checks 和 public/release docs。
