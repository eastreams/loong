# plan: Runtime / Session / Context / Crate 收敛

本文件记录 Runtime、Session、recursive execution Context 的剩余迁移边界，以及 transitional
crate 清理。
具体提交顺序见 `08-next-steps.md`。

## 当前事实

- `loong-runtime::Runtime<C>` 已经持有 `Kernel<C>` 和 erased typed `ToolPlane<C>`；这个 public
  caller-provided plane 会暴露 raw dispatch，是步骤 5 必须删除的实现偏差。
- `loong-runtime::tool_plane` 已经拥有 plane-local `ToolPath`、`ToolInvocationAction`、
  `ToolPlane` trait 和 slot-backed `ToolPlaneRegistry`。
- app bootstrap 已使用 fallible `builtin_tool_plane()` 构造 registry；不存在需要迁移的全局
  `OnceLock` tool plane。
- Context 不再保存 action payload 或 execution plane metadata；`PolicyAny` 读取
  `ActionMeta::payload()`，`KernelInvocationContext` 只约束真正读取 legacy token/pack 的方法。
- app 仍以 `Arc<AppContextInner>` 表达 runtime authority、session state 和 invocation overlay。
  `AppContextFactory::Cx<'a> = AppContext` 没有使用 GAT lifetime，仍是 owned clone 模型。
- `22bc6a3e` 删除了原本独立的 `SessionContext`，并把 session identity、lineage、workspace、
  skills、tool view 和 subagent state 合入 `AppContextInner`。这一步混淆了生命周期，必须按
  `Session` 与 Turn `Context` 的真实职责修正，不能通过继续重命名 `AppContext` 收尾。
- 截至 2026-07-13，`AppContext` 在 97 个 Rust 文件中出现 559 次，
  `AppContextFactory` 出现 78 次；这是 workspace-wide replacement，不是局部 rename。
- channel/conversation 仍大量传播 `ConversationRuntimeBinding`，provider 仍传播
  `ProviderRuntimeBinding`。这些 enum 把“advisory 权限”错误表达成“可能没有 Context”。
- `loong-runtime` crate root 只公开真实的 `runtime` / `tool_plane` owner；`RuntimeSpine`、
  one-shot/interactive/task-status projection 和 core Session re-export 已删除。
- typed policy pipeline 已支持 terminal parent/user permission decision，grant 保留完整
  `PolicyReport`，并在外部 permission await 后复查 effective capabilities。sealed grant algorithm
  已经自动记录 mandatory authorization evidence；当前 typed tool grant 仍错误地接收 legacy
  pack/token，步骤 5 必须改用 Access 已经使用的 `PolicyEngine::grant`，而不是新增 Kernel grant
  API。token expiry/revocation 继续由 legacy fallback 自己验证，不能反向塑造新 contract。
  production Context 也尚未接通 permission
  interaction，pipeline 尚未强制 permission 位于所有 hard deny 之后；这些边界完成前，production
  不得注册会返回 permission decision 的 policy。

## 目标 Ownership

### Runtime

`Runtime<C>` 是一个治理域的长期 owner：

- 持有 kernel、typed tool plane 和真正属于 runtime 的长期 registry/configuration；
- 不持有 invocation Context；
- 不把 app Session/tool namespace 塞进 kernel；
- builtin/plugin registration 在 bootstrap 完成，错误通过 `Result` 返回，不在首次调用时 panic。

### Session

Session 表达跨 Turn 稳定的 typed identity 与 baseline，但其进程内物化生命周期必须单独说明：

- 只拥有 identity、lineage、baseline capabilities、稳定 config，以及并发安全的
  lifecycle owner；Session 自身必须是 `Send + Sync`，不能混入只能由单线程/UI owner 持有的
  handle；
- 不保存 `CapabilityToken`、pack、token evidence 或本次 invocation Context。legacy bearer
  evidence 只留在旧 ingress/fallback owner，不能成为 typed Session 的持久化或恢复内容；
- Rust lifetime 只约束 Context 借用，不负责注销、取消、恢复或持久化 Session；
- durable Session record 与进程内活跃 owner 必须保持语义可区分，不能因为名字相同就假定是
  同一个 object lifetime。
- 现有 `loong_core::Session` 是 workspace/task/artifact 领域聚合，不是 app authority owner；
  不能把它机械复用为这里的 Session。若最终公开路径产生概念冲突，应在对应 owner 迁移中
  破坏性解决命名，而不是增加 alias。
- 第一阶段允许在每个 Turn 开始时从 repository snapshot 物化一个 owned Session。这只是同一
  durable identity 的新内存快照，不表示同一个 active object 跨 Turn 存活；当前没有通用
  per-session serialization、passivation 或不可重建资源，因此 Session registry 和永久 actor
  都不是 Context 迁移的前提。
- Context 通过共享引用读取 Session 的 typed baseline/config。需要在 Context 存活期间更新的
  lifecycle state 必须封装在 Session 的并发安全 owner 中，并通过窄方法更新；Context 不持有
  `&mut Session`，也不把 lifecycle snapshot 缓存成第二份状态。SQLite connection、UI/provider
  handle 不进入 Context，否则 `Context<'a>: Send` 与跨 `.await` 更新会产生错误耦合。

### Context

`Context<'a>` 是递归执行作用域的不可变视图，不等同于整个 Turn，也不固定代表某一次
invocation。Turn boundary 先构造 base Context；tool/action 的递归调用从 parent Context 派生
同类型 child Context：

```text
Session typed baseline
  + typed Turn options (mode / goal / narrowing / request options)
  + Turn cancellation signal
  -> validate and normalize
  -> base Context<'a>
  + invocation narrowing
  -> child Context<'a>
```

Context 只包含本次执行真正需要的投影：

- 对 Runtime/Session 稳定数据的借用；
- 归一化后的 mode/goal/options；
- effective capabilities、tool config 和 fs resolution/policy views；
- 本次 Turn 的 cooperative cancellation signal 的廉价 owned clone；
- 其它已经证明是本次执行属性的窄数据。

`Context<'a>` 必须实现 `Clone`，其中 base Context 的常用 clone path 必须廉价。`'a` 只证明
Runtime/Session 引用有效，不表达或管理 Session 的业务生命周期；clone 也不能成为新的 owner。
目标字段形状是：

```rust
#[derive(Clone)]
pub struct Context<'a> {
    runtime: &'a Runtime<RuntimeContextFactory>,
    session: &'a Session,
    cancellation: CancellationToken,
    mode: TurnMode,
    goal: Option<GoalId>,
    effective_capabilities: Cow<'a, Capabilities>,
    tool_config: Cow<'a, ToolRuntimeConfig>,
    fs_resolution_root: Cow<'a, Path>,
    fs_allowed_roots: Cow<'a, [PathBuf]>,
}
```

base Context 的字段使用 `Cow::Borrowed` 借用已经归一化的数据；caps/tool/root override 只把
被收窄的字段替换成 `Cow::Owned`。字段 ownership 不改变 accessor 语义：

```rust
fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
    Cow::Borrowed(self.effective_capabilities.as_ref())
}
```

base 和 child Context 都只从 accessor reborrow，绝不因读取 effective capabilities 再次 clone。
Clone base Context 只复制引用和 cancellation handle；显式 clone 已有 owned override 的 Context
才会深复制对应字段，因此 orchestration 不应靠反复 clone derived Context 派生 sibling。只有
profiling 证明某个大型 owned view 确实需要频繁 clone 时，才为该字段引入共享存储，不能为了
理论上的 O(1) clone 把 `Arc` 铺满 Context。

Context 不包含：

- tool/action payload；
- `CapabilityToken`、pack 或其它 legacy token evidence；
- `ExecutionPlane` / `PlaneTier`；
- mailbox、task supervisor、session registry 或持久化 repository；
- 独立 kernel/policy/audit owner；
- 仅为了满足 `'static` 而复制的 `Arc<AppContextInner>`。

构造规则：

- Context 构造是 authority normalization boundary。requested caps、tool view、roots 和其它
  override 在这里与 Session baseline 求交/校验，不能在 tool helper 中临时拼装。
- base Context 的 mode/goal/options 在执行期间不可变。nested tool invocation 派生 child
  Context 时只允许缩窄 effective caps/tool/root view，并继承 Turn cancellation。
- batch tool invocation 为每个并发分支派生 sibling Context，再 join futures；Context 没有供分支
  共享修改的 mutable overlay。
- subagent 创建新 Session，并从该 Session 构造自己的 base Context。detached agent/task 向
  Runtime 请求托管，不能通过 clone 当前 Context 获得独立 lifecycle。
- borrowed Context 只用于一个结构化 Turn 调用链。detached `spawn` 必须 move Runtime、Session
  与 owned Turn options，并在新 future 内重新构造 Context；不能让 `&Context` 逃逸为
  `'static` task。
- `Context::access()` 是 `AccessCx::new(...)` 的唯一 app concrete 构造点；普通调用点使用
  `ctx.access()`。
- `Context::tool(path)` 是薄入口，只把 parent Context 与 path 交给 Runtime 创建借用型
  invocation handle；lookup、child derivation、grant、dispatch 和 audit 属于 runtime
  `ToolInvocation`。
- `RuntimeContextFactory` 只有 GAT：

```rust
impl ContextFactory for RuntimeContextFactory {
    type Cx<'a> = Context<'a>;
}
```

它不提供 factory method；value construction 属于 Session/turn orchestration。

runtime-owned `ToolInvocation` 与 app-defined Context 之间只有一个直接 contract：

```rust
pub trait ToolInvocationContext: Sized {
    fn derive_tool_child(
        &self,
        capabilities: Capabilities,
    ) -> Result<Self, CapabilityNarrowingError>;
}
```

`ToolInvocationContext` 属于 `loong-runtime`，app concrete `Context<'a>` 实现它。trait 只表达
“从 parent 按给定 capabilities 派生同类型 child Context”，不携带 Factory 参数或 associated
error，不暴露 kernel、audit 或 Runtime，不进入 `ContextFactory`，也不能构造 base Context。
附近注释必须说明它解决的是跨 crate child authority narrowing，而不是为了缩短调用写出的搬运
helper。

### Permission Authority

Permission 是 policy terminal decision 后的 consent 流程，不是第二套 authorization：

- `RequireParentPermission` 请求当前 Session 的 parent；parent 可以把请求升级给 user。
- `RequireUserPermission` 请求 Session 树之外的 root user actor；user 没有更高 authority，不能
  再升级。
- approved 不能增加 capabilities、覆盖 hard deny、放宽 root/runtime limit，或替换原始
  `PolicyReport`。typed grant 在 permission await 返回后复查 effective capabilities；legacy
  fallback 继续单独复查其 token expiry/revocation。
- permission decision 只能在全部 hard authorization policy 之后成为 terminal outcome。
  pipeline registration 必须用类型/阶段编码区分 hard constraint 与 terminal consent，并同时支持
  typed `Policy` 和 `PolicyAny`。constraint stage 只接受 `Deny` / `Continue`；`Allow`、permission
  或 `Advance` 都必须作为结构化 misconfiguration deny，不能靠人工注册顺序约定正确性。
- 不增加 `PermissionContext`、permit token、`SessionAuthority` 或其它 approval/grant wrapper。
  `Granted<A>` 已由私有构造保证不可伪造；`PolicyContext` 的 async hooks 直接接收 action/report，
  由 concrete Context 通过 Runtime/Session orchestration 路由。
- `PolicyContext` 只读返回 owned typed authorization subject/identity，供 mandatory authorization
  audit attribution；它不暴露 audit sink、clock、id source 或 `ctx.audit`。
- hooks 默认返回结构化 `PermissionRequestError::Unavailable`。没有 permission surface 的
  test/fixture 与 production Context 都 fail closed，不通过默认 panic 区分接线状态。
- permission request/resolution 与普通 grant 一样自动进入 generic authorization audit；core grant
  algorithm 在 capability gate 前分配 authorization attempt id。capability deny 使用不带 report 的
  attempt event；policy 一旦运行，permission interaction 与 terminal outcome 都携带同一 report 和
  attempt id。interaction 明确区分 requested、approved、denied、parent-to-user escalation 和 failed；
  user authority 没有 escalation event。最终 allow 另行分配 grant id。policy、tool、Access backend
  都不手写这类 evidence。

## Context 破坏性替换

替换不能机械保留旧字段：

1. 将 `AppContextInner` 字段按 Runtime、Session、Turn option、Context derived view、Action
   payload 五类重新归属。
2. 删除 `Deref` / `DerefMut` / `Arc::make_mut`、`child`、`for_session`、`for_invocation` 等旧 COW
   mutation API。新 Context 构造/派生必须显式表达 Turn options 或 authority narrowing。
3. advisory Session 仍构造同一种 Context，只是 Session baseline authority 没有 `InvokeTool`
   或 mutation caps。删除 `Option<AppContext>`、`ConversationRuntimeBinding::AdvisoryOnly` 和
   provider no-context 对应物，不能用“没有 Context”表达权限模式。
4. 一次性迁移 production、tests、fixtures、trait impl、generic instantiation、注释和文档中的
   concrete 名称，然后删除三个 `AppContext*` 类型；不留 compatibility alias。

### 可独立提交与原子边界

`PolicyEngine::grant` 的 mandatory authorization audit owner 已闭合。在 workspace-wide 类型替换
前，剩余可独立编译并直接减少错误耦合的前置边界是：

1. 由 runtime `ToolInvocation` owner 同时接入 composite typed error、child narrowing、direct
   `PolicyEngine::grant`、granted dispatch 和 execution audit；wrapper 保留 outer
   `ActionGrant.id/info` 直到关联 execution audit 结束，通过 Kernel 现有 generic
   `record_audit_event` 写入 evidence。删除 app-owned wrapper 与 tool-specific
   `Kernel::record_tool_invocation`，保留 generic recorder；
2. channel/gateway 等长期 owner 只保留 Runtime，在 session address 确定后才物化当前 Session；
   provider、core tool 与 app tool 不能同时使用 outer/root context 和 session-specific context。

之后的 `Session + Context<'a> + RuntimeContextFactory` 是一个不可再拆的类型替换：

- 同一切片定义 owned Session、borrowed Context 和 GAT factory；
- async/background owner 保存 Runtime/Session，在 future 内构造 Context；
- 同一切片删除 `AppContext*`、COW API 和 RuntimeBinding 双路径。

不能通过 `Cx<'a> = &'a AppContext`、`Arc<Session>` 包装旧 Context、双 Context adapter 或
blanket forwarding trait 拆小该切片；这些做法只会把错误 owner 固化成兼容层。

### 模型不保证的事情

- Context 不会自动串行化同一 Session 的并发 Turn。需要线性化时，必须由 repository
  version/CAS、lease 或有真实需求的 active-session supervisor 提供。
- Context lifetime 不负责跨进程唯一激活、crash recovery 或恢复旧 token。恢复时 authority
  必须根据当前 config/policy 重新建立，不能反序列化旧 bearer evidence。
- cancellation 只能保证观察到信号后不再调度新的 action，不能回滚已经提交的 side effect，
  也不能消除检查与 grant 之间的竞态。
- Runtime/Session shutdown 若需要按 id 取消并等待活跃 Turn，必须有真实的 task owner 保存
  cancellation handle 与 join handle；root cancellation signal 本身不能替代监督和 finalization。
- GAT 只传播在 core/kernel/access/tool/runtime 治理链。conversation、provider 和 repository
  使用 app concrete `Context<'_>`，不继续泛型化，也不承诺 `ContextFactory` object-safe。

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
- `loong-runtime`：已经拥有 `Runtime<C>` 和 ToolPlane primitive；旧 projection spine 已删除，
  runtime owner 保留并继续承接 Session/Context。
- `loong-contracts` / `loong-core`：继续按稳定 data 与 behavior contract 分工。

剩余收敛候选：

1. 审计 `loong-cli` 与 `loong-app-protocol`。如果只是 transitional CLI/protocol forwarding
   shell，合并到 daemon 或真实 protocol owner。
2. 修正 kernel -> `loong-plugin-sdk` 的反向依赖。kernel 所需 contract 下沉到 leaf；SDK 只保留
   plugin author-facing API。
3. 审计 `protocol` / `bridge-runtime` 是否拥有稳定 wire/bridge primitive；只转发的壳合并，
   真实协议边界保留。

workspace 当前 15-crate DAG 已从 `cargo metadata --no-deps` 重建，并同步到镜像文档与
fail-closed architecture check。后续 manifest 变更必须在同一提交继续同步这些事实。
