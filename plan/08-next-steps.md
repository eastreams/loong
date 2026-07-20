# plan: 最小提交顺序

本文件只列尚未完成的有序迁移目标。提交拓扑由唯一 owner 和可验证边界决定：步骤 5 是
runtime-owned execution boundary，步骤 7 是 workspace-wide 原子替换；其它目标不把一个编号机械
塞进单个 commit。目标完成后删除该项，长期不变量只保留在 `01` 到 `07`。

## 当前 active goal

剩余步骤 5 到 8 共同完成这条破坏性 typed execution spine：

```text
Runtime
  -> owned Session
  -> borrowed Context<'a>
  -> PolicyEngine::grant + mandatory authorization audit
  -> ActionGrant<A>
  -> Granted<A>
  -> runtime ToolInvocation / Access
```

这四步的共同边界：

- `PolicyEngine::grant` 是唯一 typed authorization API；core grant algorithm 对外不可覆写，且在
  terminal authorization write 成功前不能 mint。runtime typed tool 必须直接复用这条已闭合路径。
- 禁止新增 `Kernel::grant`、`SessionAuthority`、permit token、`AuditHandle`、route/receipt、
  `ctx.audit` 或 compatibility alias/wrapper。
- typed Session 只保存 identity、lineage、baseline capabilities、稳定 config 和 lifecycle state；
  `CapabilityToken`、pack 与 token evidence 只留在旧 ingress/fallback。
- `ContextFactory` 只有 GAT；effective capabilities API 保持
  `Cow<'_, Capabilities>`，child Context 只能收窄 authority。
- 物理副作用只发生在 Access。nested typed tool 只能编排，最终仍必须进入
  `Granted<ConcreteAction>` 的 Access operation。
- typed tool primitive 已有 `RegisteredToolError` 与 runtime
  `error::{RegistrationError, LookupError, ToolInvocationError}`。`ToolPath` 由 contracts 固定为
  canonical segmented identity，runtime plane 只决定索引结构。composite `ToolInvocationError` 由
  runtime wrapper 使用并保留 concrete source；`PolicyGrantError` 不降级成 `KernelError` 或字符串。
- `ctx.tool(...).invoke(...)` 是普通 caller 唯一入口；raw granted ToolPlane dispatch 只在
  `loong-runtime` 内可达。
- legacy fallback 只在 typed path 未注册时发生；override/narrowing/grant/parse/dispatch error
  都不 fallback。

filesystem 模块收敛、streaming cancellation、全部 legacy tool 迁移、fs TOCTOU、crate 收敛、
bitset 与 generic Access execution evidence 不属于当前 active goal，分别列在步骤 9、14、15、
18、19、20、21。步骤 7 只建立
Context 的 cancellation field 与 inheritance；provider/Access observation、gateway wiring 和
finalization behavior 留在步骤 14。

## 5. 让 runtime ToolInvocation 成为强制 execution audit 边界

**范围**

- 将 typed `ToolInvocation` wrapper 从 app 迁入 `loong-runtime`。`ctx.tool(path)?.invoke(payload)` 是
  普通 caller 唯一入口，同一个 runtime owner 绑定 lookup、caps override、child narrowing、direct
  grant、granted dispatch 和 execution audit；删除 app-owned `ToolInvocation`，不保留 forwarding
  wrapper。
- 在 runtime wrapper 接线的同一提交中定义并实际使用 composite `ToolInvocationError`。它分别
  保留 `LookupError`、invalid caps override、`CapabilityNarrowingError`、`PolicyGrantError`、
  `DispatchError` 和 typed `AuditError` source；不先提交未接线的 public error scaffold。
- 在 `loong-runtime` 定义唯一直接 requirement trait `ToolInvocationContext`。它只按给定
  `Capabilities` 从 parent 派生同类型 child，`derive_tool_child` 返回
  `Result<Self, CapabilityNarrowingError>`；trait 没有 Factory 参数或 associated error，不暴露
  kernel、audit 或 Runtime，不放进 `ContextFactory`，也不构造 base Context。app concrete Context
  直接实现它。
- `Context::tool(path)` 保持薄入口，只调用 Runtime 创建 handle；runtime `ToolInvocation` 通过
  `ToolInvocationContext` narrowing 后直接调用具备 mandatory audit 的 `PolicyEngine::grant`，再完成
  internal plane dispatch 和 execution audit。trait 附近注释说明它是
  跨 crate child authority narrowing contract，不是搬运 helper。
- 删除 typed caller 的 pack/token 参数和 `Kernel::grant_action`；该方法没有 legacy production
  caller，不能留到步骤 17。authorization deny/audit failure 原样保留 `PolicyGrantError` source。
- raw ToolPlane granted dispatch 收为 runtime-internal；public/普通 caller、app helper 和
  concrete `ToolImpl` 都不能直接消费 `Granted<ToolInvocationAction>` 绕过 wrapper。破坏性删除 public
  `ToolPlane::invoke`、caller-provided plane 的 `Runtime::new<P>` 以及返回 `dyn ToolPlane` 的
  `Runtime::tools()`；runtime-internal trait 只保留 crate 内 storage strategy 替换能力。catalog/spec
  查询通过不暴露 dispatch capability 的 Runtime API 提供。
- wrapper 在消费 `ActionGrant.granted` 前保留 outer `ActionGrant.id/info`，直到关联 execution audit
  结束，并用 outer id 关联 outcome；dispatch 前先完成必要的 execution-start write，该 write 失败时
  返回 typed error，且不得 dispatch。authorization deny 没有 execution event。
- dispatch 后 wrapper 必须写 terminal completed/failed/input-error/cancelled outcome。terminal audit
  failure 返回 typed `ToolInvocationError`，但 execution 可能已经 completed 或产生 side effect，
  因此不得自动重试。
- dispatch success + terminal audit failure 的 error variant 同时保留“execution completed”事实和
  typed audit source；dispatch failure + terminal audit failure 的复合 error variant 同时保留
  dispatch 与 audit 两个 typed source，不能互相覆盖。
- runtime 不得访问 kernel-private audit state。wrapper 通过 Kernel 现有 generic
  `record_audit_event` governance recorder 提交 execution evidence；保留该 recorder，并将其 error
  boundary 收敛为 typed `AuditError`。recorder 负责 clock、event id 与 sink write。
- `record_audit_event` 附近用短注释说明其真实 ownership 职责，因此它不是 forwarding helper；不新增
  `AuditHandle`、route/receipt、`ctx.audit` 或同义 capability。`ToolImpl` 不获得 audit API。
- 删除 tool-specific `Kernel::record_tool_invocation`、tool-specific route/registry schema 和
  `TODO(tool-audit-owner)`；保留并收敛 generic `record_audit_event`。legacy `PlaneInvoked` 只随对应
  legacy plane 保留。
- 只有 `LookupError::NotRegistered` 可以交给 legacy ingress fallback；override、narrowing、grant、
  audit、parse/input、dispatch 和 concrete execution error 都不 fallback。

**完成线**

- 只有 sink 成功接受 terminal write，才存在一条带 grant id 的 terminal execution evidence；
  audit write failure 必须显式传播，不能伪造 terminal evidence；
- dispatch 前必要 audit write 失败时 dispatch count 为零；dispatch 后 terminal audit 失败不触发
  自动重试；
- tests 覆盖 pre-dispatch audit failure、success + audit failure、dispatch + audit failure；断言第一种
  不 dispatch，第二种保留 completed 事实，第三种同时保留两个 typed source；
- raw granted dispatch 在 `loong-runtime` 外不可调用；
- `Runtime` 不接受外部 `ToolPlane` implementation，也不返回带 granted dispatch capability 的 trait
  object；内部 trait 不能成为绕过 wrapper 的扩展面；
- runtime `ToolInvocation` 与 app Context 之间除 `ToolInvocationContext` 外没有第二个 direct
  bridge；`ContextFactory` 仍只有 GAT；
- composite `ToolInvocationError` 的每个 variant 都有真实 runtime caller，没有 public scaffold；
- app-owned `ToolInvocation`、`Kernel::grant_action` 与 `Kernel::record_tool_invocation` 均已删除，
  production 和 tests 都没有 caller；
- runtime 没有 sink/clock/event-id state access；跨 crate audit 只经过返回 typed `AuditError` 的
  `Kernel::record_audit_event`；
- typed execution audit 只引用 contracts-owned ToolPath/GrantId，不记录 registry representation、
  legacy route 或 pack/token；
- concrete tool 无法跳过 wrapper，也不手写 audit；本步骤不声称 generic Access execution evidence
  已经存在。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交**

`refactor(runtime): own audited typed tool invocation`

这是一个 owner-driven 原子接线提交：error、wrapper、direct grant、dispatch、audit 与旧入口删除必须
同时可用，不能拆出未接线 public error commit。

**验证**

```bash
cargo test -p loong-runtime tool_plane
cargo test -p loong-app tool_invocation
cargo test -p loong -- audit
cargo fmt --all -- --check
git diff --check
```

## 6. 消除 outer/root 与 session-specific 双 Context

**范围**

- channel/gateway/turn service 长期只持有共享 Runtime，不在具体 session identity 未知时构造
  host/root `AppContext`。
- session address 确定后再物化当前 session typed authority；同一 Turn 的 provider、core tool、
  app tool 和 policy 全部使用这一份 session-specific execution context。
- 修复 core tool 从 `ConversationRuntimeBinding` 取 outer context、而 preflight/app tool 使用
  session-specific context 的分流；删除对应 downstream binding 传播。
- OpenAI gateway 共享 Runtime，停止每个 request 重新 bootstrap kernel/tool plane。
- Session 可以先按 Turn 从 repository snapshot 物化；不为本步引入永久 actor、registry 或
  Context adapter。

**完成线**

- 一个 Turn 不同时持有 outer/root context 与 session-specific context；
- channel session 的 identity、workspace、narrowing 和 baseline capabilities 进入同一 typed path；
- entry surface 不重复签发同一 session authority；
- 没有为保留双路径新增 alias、optional wrapper 或 forwarding helper。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. 按 channel/app owner 清除 outer context 传播。
2. 按 daemon gateway owner 共享 Runtime 并延后 Session 物化。

**验证**

```bash
cargo test -p loong-app inbound_turn
cargo test -p loong-app conversation
cargo test -p loong-app file_read
cargo test -p loong -- openai_compat
cargo check -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

## 7. 原子替换为 owned Session + borrowed Context

**范围**

- 在 app runtime boundary 定义 owned `Session`、borrowed `Context<'a>` 与
  `RuntimeContextFactory`。`ContextFactory` implementation 只有
  `type Cx<'a> = Context<'a>`，不提供 value factory method。
- Session 只保存 typed identity、lineage、baseline capabilities、稳定 config 和并发安全的
  lifecycle state；不保存 `CapabilityToken`、pack、token evidence 或 invocation Context。
- Context 从 Runtime/Session borrow、本次 Turn typed options 和 cancellation signal 构造；归一化
  mode/goal、effective caps、tool config、roots 和其它本次执行 view。
- effective capabilities 字段保持 `Cow<'a, Capabilities>`：base 字段是 `Cow::Borrowed`，child 中
  被收窄的字段是 `Cow::Owned`；`PolicyContext::allowed_capabilities()` 对两者都返回
  `Cow::Borrowed(self.effective_capabilities.as_ref())`，绝不因 accessor 再次 clone。nested
  invocation 派生同类型 child Context，只能收窄 caps/tool/root view 并继承
  mode/goal/cancellation。
- 将 `AppContextInner` 的剩余字段归属到 Runtime、Session、Turn options、Context
  derived view 或 Action payload，不保留第二份 source of truth。
- batch tool invocation 为每个分支派生 sibling Context；subagent 创建新 Session。detached task
  只 move Runtime、owned Session 和 Turn options，并在 future 内重建 Context；`&Context` 不逃逸
  到 `'static` task。
- advisory Session 也构造同一种 Context，只通过 baseline capabilities 表达限制；删除
  `ConversationRuntimeBinding` / `ProviderRuntimeBinding` 的 no-context 分支。
- 新 app `Context<'a>` 直接实现既有 runtime-owned `ToolInvocationContext`；
  `Context::tool(path)` 仍是薄入口。不得为原子替换增加 Context adapter、第二个 bridge 或
  `RuntimeContextFactory` 方法。
- 一次性迁移 production、tests、fixtures、generic instantiation、注释和文档，删除
  `AppContext`、`AppContextInner`、`AppContextFactory`、旧 COW mutation API 和 compatibility
  re-export。同一提交同步 `plan/02-runtime-and-crates.md` 的当前事实、
  `plan/07-kernel-audit-and-deviations.md` 的对应“当前偏差”和 Code TODO 对照。

**完成线**

- `ContextFactory::Cx<'a> = Context<'a>` 真正使用 lifetime；
- Runtime 是长期 owner，Session 是 Context 之外的 owned materialization；Context 不含
  `Arc<AppContextInner>`、pack/token evidence 或独立 policy/audit owner；
- base Context clone 只复制 Runtime/Session 引用、borrowed Cow field 和 cancellation handle；child
  effective capabilities accessor 也只 reborrow owned field，不 clone；
- `ctx.access()` 与 runtime-owned `ctx.tool(path)?.invoke(payload)` 是普通执行入口；
- `RuntimeContextFactory` 命名不变且仍只有 GAT；app Context 直接实现
  `ToolInvocationContext`，没有 adapter；
- 删除本步骤时已经同步改写 `plan/02-runtime-and-crates.md` 当前事实、删除/改写
  `plan/07-kernel-audit-and-deviations.md` 对应“当前偏差”，并更新 Code TODO 对照；
- 以下搜索无输出：

```bash
rg -n "AppContext|AppContextInner|AppContextFactory" crates docs AGENTS.md CLAUDE.md ARCHITECTURE.md
rg -n "ConversationRuntimeBinding|ProviderRuntimeBinding" crates/app/src crates/daemon/src
```

**验证**

```bash
cargo test -p loong-app context
cargo test -p loong-app conversation
cargo test -p loong-app file_read
cargo test -p loong-kernel access
cargo check -p loong-core -p loong-runtime -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

这是唯一允许的大原子提交。不能用 `Cx<'a> = &'a AppContext`、`Arc<Session>` 包旧 Context、双
Context adapter、alias 或 blanket forwarding trait 拆小。

建议提交：`refactor(app): replace app context with recursive execution context`

## 8. 完成 typed/legacy quarantine

**范围**

- 持有 Context 的 caller 直接使用 `ctx.tool(path)?.invoke(payload).await`；删除
  `execute_kernel_tool_request` 中只搬运 typed payload/outcome 的 bridge。
- typed-first ingress 只在 `ToolInvocationError` 明确表示 path 未注册时进入 legacy fallback；
  override、narrowing、authorization deny、audit failure、parse/input 和 execution error 原样返回。
- typed Session/Context/Runtime ToolInvocation 不再暴露 pack/token。legacy bearer evidence、
  `KernelInvocationContext` 和 token validation 只留在旧 ingress/fallback module 的 bounded impl。
- typed registration 的 descriptor/path/output metadata 只来自 concrete tool + runtime plane；legacy
  static catalog 只描述尚未迁移的 legacy tools，不能覆盖 typed registration。
- 尚未迁移的 concrete tools 继续明确留在 legacy plane，不注册进 typed plane冒充完成；全部逐
  tool 迁移与最终 envelope 删除留到步骤 15。

**完成线**

- typed caller 不构造 `ToolCoreRequest` / `ToolCoreOutcome`，不调用 legacy adapter；
- raw ToolPlane dispatch、pack/token 和 legacy error conversion 不出现在 typed Session/Context path；
- typed path 命中后任何失败都不 fallback；
- `rg -n "ToolCoreRequest|ToolCoreOutcome|KernelInvocationContext" crates/app/src/context.rs \
  crates/loong-runtime/src` 无输出；剩余命中只允许位于明确的 app/kernel legacy ingress module。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**验证**

```bash
cargo test -p loong-runtime tool_plane
cargo test -p loong-app tools
cargo test -p loong-app conversation
cargo check -p loong-runtime -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

## 后续独立目标

以下步骤不属于当前 active goal。每项在开始前重新核对 owner 和 caller，不得借后续目标扩大
剩余步骤 5 到 8 的提交。

## 9. 按 filesystem operation 共置 Access 实现

**范围**

- 将 filesystem concrete action、options、`FsAccess` method、granted execution 和 output 按
  operation 共置到 `read.rs`、`write.rs`、`glob.rs` 等文件；不再让修改一个 operation 横跨
  `action.rs`、`access.rs` 和 output/execution 文件。
- `access.rs` 只保留 `FsAccess` identity、构造和强制 resolve -> path grant 顺序的共享 chain；
  `action.rs` 只保留 `FsAction` family；`path.rs` 只拥有共享 path facts、resolve/path actions 与
  execution；`error.rs` 只拥有跨 operation 共用的 fs domain error。
- operation modules 保持 private，由 `fs` facade 显式 re-export public members。破坏性删除
  `fs::action::*` 和其它内部 module path，不保留 alias、forwarder 或 compatibility re-export。
- 保持现有 authorization、path typestate、副作用顺序和 error source 不变。不要把本次文件 ownership
  调整与 authorization-audit、新 Access primitive 或 TOCTOU backend 混进同一提交。
- operation 边界只加说明授权/副作用 ownership 的简短注释；tests 继续按 operation 放在
  `fs/tests/<operation>.rs`，不重新聚合成大测试文件。

**完成线**

- concrete action definitions 不再集中在 `action.rs`；一个 operation 的主要实现可以在单个文件内
  阅读和修改；
- `access.rs` 不保存 concrete operation method 或 final side effect；
- `rg -n '^pub mod ' crates/access/src/fs.rs` 无输出，workspace caller 不再使用旧 module paths；
- 没有仅为搬运相同参数或维持旧路径新增的 helper/alias；行为测试保持不变。

**最小提交**

`refactor(access): co-locate filesystem operations`

**验证**

```bash
cargo test -p loong-access
cargo test -p loong-kernel access --all-features
cargo clippy -p loong-access -p loong-kernel --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## 10. 将 provider/runtime-self live source 完全迁入 Access

**范围**

- provider source loader 只接收 `&Context<'_>`，通过 `ctx.access().fs()` 读取
  `AGENTS.md` / `TOOLS.md` / `IDENTITY.md` 等 live source。
- candidate discovery 保持 lexical；存在性、canonical containment、symlink escape 和内容读取
  由 fs actions 决定。
- context engine 同时产出 assembled prompt 和结构化 `RuntimeSelfContinuity`；compaction 只消费
  结构化结果，不从 prompt/config 现场回读文件。
- 删除 no-context/advisory live-source fallback 和
  `TODO(deprecate-no-kernel-live-source)`。

**完成线**

- live source 内容只在 granted fs action 中读取；
- 没有 entry-level root Context 或 config fallback 现场读文件；
- prompt assembly 不伪造 tool invocation audit。

**验证**

```bash
cargo test -p loong-app workspace_guidance
cargo test -p loong-app runtime_self
cargo test -p loong-app context_engine
cargo check -p loong-access -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

建议提交：`refactor(app): govern runtime source reads through access`

## 11. 迁移 config.import skills lifecycle 副作用

**范围**

- 范围严格限于 `apply_selected + apply_skills_plan=true` 依赖的 `skills.install` /
  `skills.remove`、external manifest、staging/copy/archive/extract/index/remove 和 failure rollback。
- concrete config import tool 只 parse/compose；它可以通过 `ctx.tool(...).invoke(...)` 编排 nested
  typed skills tool，但每个文件、网络或进程 side effect 最终都必须进入对应
  `Granted<ConcreteAction>` 的 Access operation。
- 迁移完成后删除 `FilePolicyExtension`、direct file preflight 对应分支和
  `TODO(config-import-access)` / `TODO(access-migration)`。

**完成线**

- kernel-routed skills apply 不再 fail closed，也不调用 direct filesystem helper；
- legacy direct API 不再需要 `FilePolicyExtension`；
- nested tool 不成为物理副作用边界，所有 side effect 都由 Granted Access action 执行。

**验证**

```bash
cargo test -p loong-app config_import
cargo test -p loong-app skills
cargo test -p loong-access
cargo check -p loong-access -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

按 Access primitive 与 concrete tool owner 分拆提交，不把完整 skills lifecycle 塞进一个 commit。

## 12. 编码 hard constraint 与 terminal consent 阶段

**范围**

- pipeline registration 用明确阶段编码 hard constraints 与 terminal consent；两阶段保持 typed
  `Policy` / `PolicyAny` 对称。
- constraint stage 只允许 `Deny` / `Continue`；`Allow`、permission 或会跳过剩余 constraint 的
  `Advance` 产生结构化 misconfiguration denial。
- terminal consent stage 只在全部 hard constraints 通过后运行；不另造 policy engine，也不在
  pipeline 外重跑 policy。

**完成线**

- permission 只满足 consent，不增加 capabilities、不覆盖 hard deny/root/runtime limit；
- typed 与 any policy 都能显式注册到正确阶段；错误阶段的 decision fail closed；
- production 尚未接线前不注册 permission policy。

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 13. 接通 production parent/user permission interaction

**范围**

- 在 production Context 实现 `request_parent_permission` / `request_user_permission`：parent 是当前
  Session 的 parent，user 是 Session 树之外的 root actor。
- parent 可以升级给 user；user 必须 terminal。interaction unavailable 返回结构化错误，不能
  改写成普通 deny 或 transport string。
- Runtime/Session orchestration 把请求送到真实 interaction surface；Context 不保存 UI/provider
  handle，permission 也不产生 permit token。

**完成线**

- unavailable 保留原始 report 并返回 `PermissionRequestError::Unavailable`；
- parent/user 路由与 Session lineage 一致，user escalation 被结构化拒绝；
- production 只在 fail-closed permission foundation 以及步骤 7、12 完成后注册 permission
  policy。

**验证**

```bash
cargo test -p loong-app permission
cargo test -p loong-app session
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 14. 让 streaming Turn 协作取消并保证 finalization

该目标固定按 owner 拆成五个最小提交，不能合成一个大 cancellation commit：

1. **signal + tool scheduling observation**：Turn execution owner 引入 live cancellation signal 并
   放入 Context；tool scheduling 和下一次 action grant 在启动前观察它。定向测试触发 signal 后
   断言不再调度 nested tool，也不再请求下一 action grant；本提交必须有可观察行为，不能只传播
   signal。
2. **provider observation**：provider stream read/retry/backoff 在安全点观察同一 signal，drop
   upstream response stream 并协作退出。
3. **Access observation**：long-running search/glob/read-dir 等 Access operation 在各自安全点观察
   同一 signal；不把 cancellation 变成 policy input。
4. **gateway trigger**：SSE receiver close 触发当前 Turn signal；gateway 保存自己创建的 task/join
   owner，不再 detached 地放任完整 Turn 继续执行。本提交不先引入无 evidence 的 force abort。
5. **finalization/evidence**：把 cancellable execution 与不可跳过的 finalization 分开，记录
   cancelled/timeout、partial-output policy 和 Session lifecycle transition；等待 grace period 后
   才允许 force abort，并记录 timeout。partial text 不写成 completed reply。

已经进入 backend 的 side effect 不承诺回滚。步骤 14 只记录其已有 owner 能证明的 tool/Turn
outcome；generic Access action execution evidence 在步骤 21 完成前不能假定存在。不为取消引入
永久 Session actor、全局 registry 或 Context-owned task supervisor。

**完成线**

- downstream disconnect 停止 provider stream，并且不启动后续 tool/action；
- 当前 Turn cancelled 后 Session 可以继续下一 Turn；
- forced abort 只发生在 grace timeout 后，并与正常 cooperative cancellation 区分；
- 首个提交的测试证明 signal 会阻止后续 tool scheduling 与下一 action grant；
- provider、Access、gateway 和 finalization 各自在 owning boundary 有定向取消测试；五个提交都可
  独立验证。

**验证**

```bash
cargo test -p loong-app streaming
cargo test -p loong-app cancellation
cargo test -p loong -- openai_compat_stream
cargo fmt --all -- --check
git diff --check
```

## 15. 逐个迁移全部 legacy tools 并删除 tool envelope

**范围**

- 先列出每个仍由 `ToolCoreRequest` / `LegacyToolPlane` 服务的 concrete tool；每个 tool 使用独立
  提交迁入 typed registration，不能用一个机械大提交迁完整个 catalog。
- 每个 tool 提交同时完成 typed input/output、`ToolImpl` registration、必要的 nested orchestration
  和 Access action；随后删除该 tool 的 legacy adapter/caller。
- fallback 只用于尚未注册的 tool path。某个 tool 注册后，其 parse/deny/execution error 不再进入
  legacy 分支。
- caller 全部清空后，单独删除 `ToolCoreRequest` / `ToolCoreOutcome`、`Kernel::execute_tool_core`、
  `LegacyToolPlane`、`CoreToolAdapter` / `ToolExtensionAdapter`、static catalog duplicate metadata、
  display alias 和对应 TODO。

**完成线**

- 新增 builtin tool 只需要 concrete `ToolImpl` 和一条 registration；
- 每个 migrated tool 的物理 side effect 只在 Access；
- legacy tool envelope 无 production caller，最终清理提交的搜索无输出；
- 没有用 compat alias/wrapper 保留旧 API。

**验证**

```bash
cargo test -p loong-runtime tool_plane
cargo test -p loong-app tools
cargo test -p loong-app conversation
cargo check -p loong-contracts -p loong-core -p loong-runtime -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

## 16. 逐 plane 迁移并删除 non-tool legacy execution paths

每个 plane 都按“typed action/Access contract -> caller migration -> legacy adapter/envelope deletion”拆成
独立最小提交；前一个 plane 清空后再处理下一个：

1. **control plane**：定义 typed action，改用 `PolicyEngine::grant`，删除 explicit legacy allow
   bootstrap 与 `TODO(control-plane-action)`。
2. **memory plane**：为 load/store/search 等实际操作建立 typed action 和 Access boundary，迁移
   `MemoryCoreRequest` / extension caller，再删除 legacy memory adapters。
3. **connector plane**：为 connector operation 建立 typed action 和 Access boundary，迁移 core /
   extension caller，再删除 legacy connector adapters。
4. **runtime plane**：为 runtime operation 建立 typed action 和 Access boundary，迁移 core /
   extension caller，再删除 legacy runtime adapters。
5. **harness plane**：为 task/harness execution 建立 typed action 和 owning Access boundary，迁移
   broker caller，再删除 legacy harness route/envelope。

typed policy grant 只授权 action；文件、网络、进程或外部系统副作用最终仍由各 domain Access
执行。不得用统一 forwarding facade 包住旧 plane，也不得在一个提交里删除多个 plane。

**完成线**

- 每个 non-tool plane 都有明确 caller-zero 证据后才删除旧 surface；
- production 不再通过 legacy plane 或 `authorize_operation` 发起 governed execution；
- legacy authorization surface 的剩余 caller 只可能是步骤 17 明确审计的 dead API/test fixture。

**验证**

```bash
cargo test -p loong-kernel
cargo test -p loong-app
cargo test -p loong -- control_plane
cargo fmt --all -- --check
git diff --check
```

## 17. 删除 legacy kernel authorization surface

**范围**

- `Kernel::grant_action` 必须已经随步骤 5 删除；它没有 legacy production caller，不能等待步骤
  15、16。这里不再迁移或保留它。
- 在步骤 15、16 的 caller 全部清空后，删除 `authorize_operation`、`policy_engine_error` 和 legacy
  `PolicyError` conversion。
- 删除不再有 ingress owner 的 `KernelInvocationContext`、token/pack authorization methods 与 bearer
  evidence type；若某个 wire contract 仍有真实 caller，先把该 caller 纳入步骤 15 或 16，不能
  留 fallback/workaround。
- 保留现有最小 `loong_core::kernel::Kernel<C>` Access contract；没有真实外部需求时删除
  `TODO(kernel-contract)`，不新增宽 forwarding trait。

**完成线**

- production 与 tests 都不调用 legacy authorization API；
- typed authorization error/report 到 owning boundary 前不降级成字符串或 extension error；
- kernel 不存在第二套同义 governance trait、compat alias 或 token-shaped typed path。

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong-app tools
cargo test -p loong -- control_plane
cargo check -p loong-core -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

## 18. 用 descriptor-relative backend 关闭 fs TOCTOU

**范围**

- 先记录跨平台 backend/library 决策，再实现 descriptor-relative/capability-based fs primitive。
- `GrantedFsPath` 携带或引用 backend 可消费的稳定 handle；最终 operation 不重新解析已授权
  `PathBuf`。
- 保留 target/entry typestate；Unix/Windows 差异写入 contract 和 tests。

**完成线**

- authorization 与 side effect 消费同一 handle chain；
- 并发替换 symlink/ancestor 不能把操作重定向到 allowed roots 外。

**验证**

```bash
cargo test -p loong-access
cargo test -p loong-kernel access
cargo clippy -p loong-access -p loong-kernel --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## 19. 收敛 Runtime 之外的剩余 crates 与架构文档

**范围**

- `loong-runtime` 的旧 one-shot/interactive/task-status projection 已删除；后续不得恢复 phase spine、
  executor adapter 或 core type re-export。
- 逐个审计 `loong-cli`、`loong-app-protocol`、`loong-plugin-sdk`、`protocol`、
  `bridge-runtime`。每个提交只处理一个 owner 明确的 forwarding shell；有真实 command、wire 或 bridge
  contract 的 crate 保留并收窄。
- 修正 kernel -> author-facing SDK 的反向依赖；kernel 所需 contract 下沉，SDK 只保留 author API。
- 每次 manifest 变化都从 `cargo metadata --no-deps` 重建真实 DAG，并同步 `AGENTS.md`、
  `CLAUDE.md`、`ARCHITECTURE.md`、reader-facing docs 与 fail-closed architecture checks。

**完成线**

- 没有无 owner forwarding shell、compatibility facade 或 kernel -> author-facing SDK 反向依赖；
- workspace DAG、文档和 architecture checks 一致；
- `AGENTS.md` 与 `CLAUDE.md` 保持镜像。

**验证**

```bash
cargo metadata --no-deps
./scripts/check_architecture_boundaries.sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --all-features
cargo fmt --all -- --check
git diff --check
```

## 20. 将 Capabilities 收敛为值语义 bitset

**范围**

- 删除 `Capabilities(Option<Arc<BTreeSet<Capability>>>)`；空集就是零值，clone/copy、membership、subset
  和 intersection 都是无分配的值操作，不再用 `Option` 或共享所有权编码集合状态。
- 目标 concrete shape 是 opaque `Capabilities(u64)`，并实现 `Copy`。`contains`、`is_subset`、
  `intersection` 和 `difference` 直接做值运算，其中后两者返回 `Capabilities`，需要枚举时再显式
  `iter()`，不保留只为旧 `BTreeSet` 形状存在的 iterator API。
- capability variant、canonical name、bit mapping 和 `ALL_CAPABILITIES` 必须由 contracts-private 的
  单一 declaration 生成，不能维护会彼此漂移的 mapping 与遍历表。生成顺序保持当前 `Capability` 声明
  顺序；compile-time capacity assertion 拒绝超过 64 bit，测试同时锁定 one-hot、bit 唯一、全集无遗漏和
  full-mask iteration 可逆。declaration 旁用简短注释说明它是 authorization universe 的唯一 source；
  不把未承诺稳定的 bit position 当成持久化/wire identity。
- 保持 capability 名称/list 的序列化 contract；Policy、Action、Context 和 Access 不暴露 mask、
  index 或 word size，raw mask 不进入 serde、audit 或其它 wire format。未来内部存储超过单个 word 时
  可以在保持 opaque API 和 `Copy` contract 的前提下替换；放弃 `Copy` 必须作为新的 breaking decision。
- 删除 `Cow<Capabilities>` 以及仅为 set-backed clone 成本存在的借用分支；recursive Context 直接保存
  `Capabilities` 值，`PolicyContext::allowed_capabilities()` 按值返回，child 通过位与得到严格不扩权的新值。
- benchmark/profile 可以记录迁移收益，但不再作为修复当前表示的进入条件；本目标首先消除错误的
  ownership/集合建模。
- 这是一次 workspace-wide atomic breaking migration：contracts 表示、set-operation 返回类型、所有
  `PolicyContext` 实现和 caller 在同一提交切换，不保留 Cow/iterator compatibility API。

**完成线**

- capability gate、subset 和 intersection 语义与迁移前一致；
- base/child Context 都不因 capability clone 或 narrowing 分配；
- `rg -n "Option<Arc<BTreeSet<Capability>>>|Cow<'[^']*, Capabilities>" crates` 无 production 命中；
- serde 继续输出迁移前相同的 PascalCase 名称数组和 `Capability` 声明顺序；乱序及重复输入被
  canonicalize，未知名称、非数组与整数 mask 被拒绝；
- tests 覆盖空集、全集、重复输入、subset、intersection、difference、one-hot、bit uniqueness、mapping
  completeness、full-mask iteration、golden wire order 与 64-bit capacity assertion。

**验证**

```bash
RUSTC_WRAPPER= cargo fmt --all -- --check
RUSTC_WRAPPER= cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTC_WRAPPER= cargo test --workspace
RUSTC_WRAPPER= cargo test --workspace --all-features
./scripts/check_architecture_boundaries.sh
git diff --check
```

## 21. 设计并闭合 generic Granted Action execution evidence

该目标独立于当前 active goal。sealed grant algorithm 已保证所有 typed Action 的 authorization
evidence；步骤 5 只保证 ToolInvocation execution evidence。不能借二者声称 Access execution audit
已完成。

**范围**

- 先确定 generic `Granted<Action>` execution evidence 的唯一 owner 与调用签名，使 filesystem 与
  其它 Access action 在真正执行时记录 started/completed/failed/cancelled；在 owner 确定前不修改
  `Granted` 字段形状，也不预设 correlation carrier。
- 比较两种候选：A) 单一 execution wrapper 同时拥有 outer `ActionGrant` 并负责 audit 时，由它保留
  id/info；B) 只有当 `Granted` 必须独立跨越 execution boundary、owner 无法可靠保留 outer metadata，
  且保留 outer 会迫使多个 consumer 重复传 id 或增加 wrapper 时，才考虑将 `GrantId` 下沉并提供只读
  accessor。不得因调用方便提前选择 B，也不能用当前 ToolInvocation 的 outer-retention 方案替
  generic Access 预先定案。
- 无论选择哪种 carrier，都不把 sink、clock、report 或 authority 挂到 Context/`Granted`，不增加
  `Kernel::grant`，也不让 ToolInvocation wrapper 代替 generic Action owner。
- execution-start write 必须先成功才进入 `Action::run`；失败时 action dispatch count 为零。
  terminal write 发生在 action outcome 已知后，必须保留 concrete action error 与 audit source。
- terminal audit failure 不能抹去 completed/failed/cancelled 或 side effect already happened 的事实，
  不能自动重试 action、回滚 backend side effect，或把 audit failure 伪造成 action failure。
- cancellation 在 execution-start 前阻止新 action；进入 backend 后只按 action 的 cooperative
  cancellation/atomicity contract 完成或失败。已经发生的 side effect 不因随后收到 cancellation
  或 terminal audit failure 而被重写成“未执行”。
- `FanoutAuditSink` 仍只表示 engine 对配置 sink 的单次 write 调用，不提供跨 child sink
  transaction、rollback 或 retry。某个 child 已接受后后续 child 失败时，返回 source-preserving
  audit error 并禁止自动重跑 action。

**完成线**

- generic execution evidence 的唯一 owner/调用签名已经确定，并据此在候选 A/B 中完成明确决策；
- 每个 migrated Access action 的 authorization 与 execution event 使用选定 carrier 关联同一
  grant id，且没有同时保留两套 metadata path；
- 当前只委托 `Action::run` 的 `Granted<Action>::run(ctx)` 不再被误认为已有 audit；完成后其 owning
  consumption path 才能保证 generic execution evidence；
- tests 覆盖 pre-execution audit failure、action success + terminal audit failure、action failure +
  terminal audit failure、cooperative cancellation 和 Fanout partial acceptance；
- tests 明确断言 pre-execution failure 不运行 action，post-execution audit failure 不重试且保留
  side effect already happened，Fanout 不具有跨 child transaction 语义；
- Context/`Granted` 没有 sink，kernel 没有新增 grant forwarding API。

**验证**

```bash
cargo test -p loong-core policy
cargo test -p loong-kernel audit
cargo test -p loong-access
cargo clippy -p loong-core -p loong-kernel -p loong-access --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Code TODO 对照

- `TODO(session-owned-context)` -> 步骤 7；
- `TODO(typed-tool-authorization-audit)` / `TODO(tool-audit-owner)` -> 步骤 5；
- `TODO(deprecate-no-kernel-live-source)` -> 步骤 10；
- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 11；
- `TODO(deprecate-tool-core-envelope)` / `TODO(tool-plane)` /
  `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` /
  `TODO(typed-tool-legacy-ingress)` -> 步骤 15；
- `TODO(control-plane-action)` -> 步骤 16；
- `TODO(deprecate-legacy-kernel-auth)` / `TODO(deprecate-legacy-policy-error)` -> 步骤 17；
- `TODO(kernel-contract)` -> 步骤 17。
