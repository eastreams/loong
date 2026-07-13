# plan: 最小提交顺序

本文件只列尚未完成的有序迁移目标。每个目标按 owner 和可独立验证的边界拆成最小提交；除
步骤 7 的 workspace-wide 原子替换外，不把一个编号机械塞进单个 commit。目标完成后删除该项，
长期不变量只保留在 `01` 到 `07`。

## 当前 active goal

步骤 1 到 8 共同完成这条破坏性 typed execution spine：

```text
Runtime
  -> owned Session
  -> borrowed Context<'a>
  -> PolicyEngine::grant
  -> ActionGrant<A>
  -> Granted<A>
  -> runtime ToolInvocation / Access
```

这八步的共同边界：

- 保留现有 `PolicyEngine::grant`；禁止新增 `Kernel::grant`、`SessionAuthority`、permit token、
  route/receipt、`ctx.audit` 或 compatibility alias/wrapper。
- typed Session 只保存 identity、lineage、baseline capabilities、稳定 config 和 lifecycle state；
  `CapabilityToken`、pack 与 token evidence 只留在旧 ingress/fallback。
- `ContextFactory` 只有 GAT；effective capabilities API 保持
  `Cow<'_, Capabilities>`，child Context 只能收窄 authority。
- 物理副作用只发生在 Access。nested typed tool 只能编排，最终仍必须进入
  `Granted<ConcreteAction>` 的 Access operation。
- typed tool 的 lookup、caps override、child narrowing、grant 和 dispatch 全部使用
  `ToolInvocationError`；`PolicyGrantError` 不降级成 `KernelError` 或字符串。
- `ctx.tool(...).invoke(...)` 是普通 caller 唯一入口；raw granted ToolPlane dispatch 只在
  `loong-runtime` 内可达。
- legacy fallback 只在 typed path 未注册时发生；override/narrowing/grant/parse/dispatch error
  都不 fallback。

streaming cancellation、全部 legacy tool 迁移、fs TOCTOU、crate 收敛与 bitset 不属于当前
active goal，分别列在步骤 13、14、17、18、19。步骤 7 只建立 Context 的 cancellation field 与
inheritance；provider/Access observation、gateway wiring 和 finalization behavior 留在步骤 13。

## 1. 建立 foundation contracts

**范围**

- 在 `loong-contracts` 定义 `Capabilities` 的当前集合表示；
  `PolicyContext::allowed_capabilities()` 破坏性改为 `Cow<'_, Capabilities>`。base Context 的
  effective capabilities 字段是 `Cow::Borrowed`，child 中被收窄的字段是 `Cow::Owned`；accessor
  对两者都返回 `Cow::Borrowed(self.effective_capabilities.as_ref())`，绝不再次 clone。不增加
  capability provider/factory/helper trait。
- `PolicyContext` 的 parent/user permission 默认 hook 返回
  `PermissionRequestError::Unavailable`，删除默认 panic、clippy panic 例外和 should-panic tests；
  grant error 保留原始 `PolicyReport` 与 error source。
- 保持 `PolicyEngine::grant` 与 `Granted<A>` 私有 constructor，不增加同义 authority/proof 类型。
- 本步只建立语义 contract，不迁移 bitset；表示优化留到步骤 19 的 benchmark/profile gate。

**完成线**

- base capability 字段不分配，child narrowing 只为字段中的求交结果分配；两者的 accessor 都只
  reborrow，不 clone；
- missing capability、policy deny 和 unavailable permission 都返回 typed error，不 panic；
- core contract 不出现 `Kernel::grant`、`SessionAuthority` 或 forwarding helper。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. `fix(core): fail closed when permission interaction is unavailable`
2. `refactor(core): borrow or own effective capabilities`

**验证**

```bash
cargo test -p loong-core policy
cargo test -p loong-kernel permission
cargo check -p loong-contracts -p loong-core -p loong-kernel
cargo fmt --all -- --check
git diff --check
```

## 2. 删除 non-context fields 并隔离 legacy bounds

**范围**

- 删除 `KernelInvocationContext::request_parameters()` 与各 concrete/test context 中的副本；
  `PolicyAny` 直接读取 `ActionMeta::payload()`。
- 删除 `AppContext` 的 `plane` / `tier` 字段、getter 和 `for_invocation` 参数；legacy audit 若仍需
  route/tier，由旧调用边界显式传入，不能从 Context 偷渡。
- 拆开 concrete `Kernel<C>` impl：constructor、policy registration、typed policy/access 所需的
  普通 API 不带 `KernelInvocationContext` HRTB；只有真正读取 token/pack/legacy request 的方法
  保留该约束。
- `CapabilityToken`、token expiry/revocation、pack validation 和 legacy plane execution 保持现有
  fallback 行为；不重命名、不包装这些仍有 caller 的旧路径。

**完成线**

- Context 不再复制 Action payload 或保存无 consumer 的 execution plane metadata；
- 仅实现 `PolicyContext`、不实现 `KernelInvocationContext` 的 test Context 可以构造 Kernel、注册
  typed policy 并完成 Access grant；
- typed Kernel/Access API 不受 legacy token/pack trait bound 约束；
- legacy adapter/plane 的 expiry、revocation、pack boundary 和 fallback tests 行为不变。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. `refactor(app): remove non-context invocation metadata`
2. `refactor(kernel): isolate legacy invocation context bounds`

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong-kernel access
cargo test -p loong-app context
cargo test -p loong-app tools
cargo check -p loong-spec -p loong-kernel -p loong-app
cargo fmt --all -- --check
git diff --check
```

## 3. 收敛 typed ToolInvocation error 并直接 grant

**范围**

- 在 `loong-runtime` 定义 `ToolInvocationError`，分别表达 lookup、invalid caps override、child
  narrowing、policy grant 和 dispatch failure，并用 typed source 保留底层错误。
- `ctx.tool(path)`、`with_capabilities_override` 和 `invoke` 使用同一个 error boundary；删除
  `policy_denied: ...` 字符串分类和 `KernelError::ToolPlane` 中转。
- app typed ToolInvocation 使用现有 `PolicyEngine::grant(ctx, action)`，保留完整
  `ActionGrant { id, info, granted }`；删除 typed caller 的 pack/token 参数与
  token-shaped `Kernel::grant_action` 调用。
- lookup 只有“path 未注册”可以交给旧 ingress 决定 fallback；override、narrowing、grant、parse
  和 execution error 都直接返回 `ToolInvocationError`。
- 本步不改变 execution audit owner；步骤 4 先闭合 authorization audit，步骤 5 再迁移 runtime
  wrapper 和 execution audit。

**完成线**

- typed tool authorization 与 Access 共用现有 `PolicyEngine::grant`，不接收 pack/token；
- lookup/override/narrowing/grant/dispatch 任一失败都保留 typed source；
- `PolicyGrantError` 在 owning typed boundary 之前不转成 `KernelError`、`PolicyError` 或字符串；
- grant/parse/execution failure 不触发 legacy fallback。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. `refactor(runtime): type tool invocation failures`
2. `refactor(app): grant typed tool invocation directly`

**验证**

```bash
cargo test -p loong-runtime tool_plane
cargo test -p loong-app tool_invocation
cargo test -p loong-app tools
cargo check -p loong-core -p loong-runtime -p loong-kernel -p loong-app
cargo fmt --all -- --check
git diff --check
```

## 4. 闭合 authorization audit owner

**范围**

- `PolicyEngine::grant` 继续是唯一 typed authorization API。grant 固定调用 implementor 提供的
  mandatory audit behavior；caller 不能选择跳过，也不增加 public Kernel wrapper/forwarder。
- concrete kernel `PolicyPipeline` 使用 kernel-private shared authorization audit state，与 Kernel
  共用 audit sink、clock、authorization attempt/event identity 和 grant identity。state 不进入
  public core API，也不由 Context 持有。
- grant 在 capability gate/policy evaluation 前开始 attempt。capability failure、policy deny、
  permission failure 和 allow terminal 全部由同一 implementor 发起恰好一次 terminal write；
  permission requested/resolved 是零到多条关联同一 attempt 的 interaction event。
- allow 路径先分配 grant id，写入包含 action metadata、required caps、完整 report、attempt id 与
  grant id 的 terminal event；只有 sink 确认成功后才能构造并返回 `ActionGrant`。audit write
  failure 必须在 grant 逃逸前返回 typed `PolicyGrantError`；同一规则适用于 permission
  interaction evidence。
- deny/request/audit failure 保留 authorization outcome、sink error source 和已经产生的
  `PolicyReport`；不重新运行 policy，也不生成替代 reason。
- Context、caller、concrete policy、tool 和 Access backend 都不手写 authorization evidence。
  legacy pack/token validation 继续记录自己的 legacy evidence。

**完成线**

- 正常 sink 下，completed/denied/failed grant attempt 各有且仅有一条 terminal authorization
  event；failing sink 下，terminal write failure 返回 typed error；
- permission interaction 与最终 outcome 共享 attempt/action/report correlation；
- audit sink failure 时没有 `ActionGrant` / `Granted<A>` 逃逸；
- direct Access 与 typed tool grant 自动获得相同 authorization evidence；
- 没有 `Kernel::grant`、`ctx.audit` 或 authorization forwarding wrapper。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. `feat(core): model typed authorization audit failures`
2. `feat(kernel): audit every policy grant attempt`

第二个提交只跨越 `PolicyEngine` implementor 及其 tests 所需的 compile-safe contract 更新，不混入
tool execution audit。

**验证**

```bash
cargo test -p loong-core policy
cargo test -p loong-kernel audit
cargo test -p loong-kernel permission
cargo test -p loong-access audit
cargo fmt --all -- --check
git diff --check
```

## 5. 让 runtime ToolInvocation 成为强制 execution audit 边界

**范围**

- 将 typed `ToolInvocation` wrapper 归属到 `loong-runtime`。`ctx.tool(path)?.invoke(payload)` 是普通
  caller 唯一入口，wrapper 绑定 child narrowing、grant consumption、dispatch 和 execution audit。
- 在 `loong-runtime` 定义唯一直接 requirement trait `ToolInvocationContext<C>`。它只按给定
  `Capabilities` 从 parent 派生同一 `C::Cx<'_>` child，返回 typed narrowing error；不暴露
  kernel、audit 或 Runtime，不放进 `ContextFactory`，也不构造 base Context。app concrete Context
  直接实现它。
- `Context::tool(path)` 保持薄入口，只调用 Runtime 创建 handle；runtime `ToolInvocation` 通过
  `ToolInvocationContext<C>` narrowing 后调用现有 `PolicyEngine::grant`、internal plane dispatch
  和 execution audit。trait 附近注释说明它是跨 crate child authority narrowing contract，不是
  搬运 helper。
- raw ToolPlane granted dispatch 收为 runtime-internal；public/普通 caller、app helper 和
  concrete `ToolImpl` 都不能直接消费 `Granted<ToolInvocationAction>` 绕过 wrapper。
- wrapper 在消费 grant 前读取 grant id 和 stable path display，并在 dispatch 前完成必要的
  execution-start audit write；该 write 失败时返回 typed error，且不得 dispatch。authorization
  deny 没有 execution event。
- dispatch 后 wrapper 必须写 terminal completed/failed/input-error/cancelled outcome。terminal audit
  failure 返回 typed `ToolInvocationError`，但 execution 可能已经 completed 或产生 side effect，
  因此不得自动重试。
- dispatch success + terminal audit failure 的 error variant 同时保留“execution completed”事实和
  typed audit source；dispatch failure + terminal audit failure 的复合 error variant 同时保留
  dispatch 与 audit 两个 typed source，不能互相覆盖。
- execution audit 使用 Runtime 可达的 kernel-owned sink/clock/id state；不从
  `KernelInvocationContext`、pack/token 或 `ctx.audit` 取得 attribution。
- execution audit failure 通过 `ToolInvocationError` 保留 source；`ToolImpl` 不获得 audit API。
- 删除 `Kernel::record_tool_invocation`、tool-specific route/registry schema 和
  `TODO(tool-audit-owner)`；legacy `PlaneInvoked` 只随对应 legacy plane 保留。

**完成线**

- 只有 sink 成功接受 terminal write，才存在一条带 grant id 的 terminal execution evidence；
  audit write failure 必须显式传播，不能伪造 terminal evidence；
- dispatch 前必要 audit write 失败时 dispatch count 为零；dispatch 后 terminal audit 失败不触发
  自动重试；
- tests 覆盖 pre-dispatch audit failure、success + audit failure、dispatch + audit failure；断言第一种
  不 dispatch，第二种保留 completed 事实，第三种同时保留两个 typed source；
- raw granted dispatch 在 `loong-runtime` 外不可调用；
- runtime `ToolInvocation` 与 app Context 之间除 `ToolInvocationContext<C>` 外没有第二个 direct
  bridge；`ContextFactory` 仍只有 GAT；
- ToolSlot、registry key、legacy route 和 pack/token 不进入 typed execution audit payload；
- concrete tool 无法跳过 wrapper，也不手写 audit。
- 删除本步骤时同步删除/改写 `plan/07-kernel-audit-and-deviations.md` 中对应“当前偏差”，并更新
  Code TODO 对照。

**最小提交顺序**

1. `refactor(runtime): own typed tool invocation wrapper`
2. `feat(runtime): audit granted tool execution`

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
- 将步骤 2 清理后剩余的 `AppContextInner` 字段归属到 Runtime、Session、Turn options、Context
  derived view 或 Action payload，不保留第二份 source of truth。
- batch tool invocation 为每个分支派生 sibling Context；subagent 创建新 Session。detached task
  只 move Runtime、owned Session 和 Turn options，并在 future 内重建 Context；`&Context` 不逃逸
  到 `'static` task。
- advisory Session 也构造同一种 Context，只通过 baseline capabilities 表达限制；删除
  `ConversationRuntimeBinding` / `ProviderRuntimeBinding` 的 no-context 分支。
- 新 app `Context<'a>` 直接实现既有 runtime-owned
  `ToolInvocationContext<RuntimeContextFactory>`；`Context::tool(path)` 仍是薄入口。不得为原子替换
  增加 Context adapter、第二个 bridge 或 `RuntimeContextFactory` 方法。
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
  `ToolInvocationContext<RuntimeContextFactory>`，没有 adapter；
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
  tool 迁移与最终 envelope 删除留到步骤 14。

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
步骤 1 到 8 的提交。

## 9. 将 provider/runtime-self live source 完全迁入 Access

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

## 10. 迁移 config.import skills lifecycle 副作用

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

## 11. 编码 hard constraint 与 terminal consent 阶段

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

## 12. 接通 production parent/user permission interaction

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
- production 只在步骤 1、4、7、11 完成后注册 permission policy。

**验证**

```bash
cargo test -p loong-app permission
cargo test -p loong-app session
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 13. 让 streaming Turn 协作取消并保证 finalization

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

已经进入 backend 的 side effect 不承诺回滚，完成或失败后仍保留 execution evidence。不为取消
引入永久 Session actor、全局 registry 或 Context-owned task supervisor。

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

## 14. 逐个迁移全部 legacy tools 并删除 tool envelope

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

## 15. 逐 plane 迁移并删除 non-tool legacy execution paths

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
- legacy authorization surface 的剩余 caller 只可能是步骤 16 明确审计的 dead API/test fixture。

**验证**

```bash
cargo test -p loong-kernel
cargo test -p loong-app
cargo test -p loong -- control_plane
cargo fmt --all -- --check
git diff --check
```

## 16. 删除 legacy kernel authorization surface

**范围**

- 在步骤 14、15 的 caller 全部清空后，删除 `Kernel::grant_action`、
  `authorize_kernel_action`、`authorize_operation`、`policy_engine_error` 和 legacy `PolicyError`
  conversion。
- 删除不再有 ingress owner 的 `KernelInvocationContext`、token/pack authorization methods 与 bearer
  evidence type；若某个 wire contract 仍有真实 caller，先把该 caller 纳入步骤 14 或 15，不能
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

## 17. 用 descriptor-relative backend 关闭 fs TOCTOU

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

## 18. 删除 runtime transitional spine 并收敛 crates/docs

**范围**

- 删除/迁移 `loong-runtime` crate root 的 `RuntimeSpine`、one-shot/interactive phase API 和无 owner
  re-export，保留 `Runtime<C>` / ToolPlane owner。
- 逐个审计 `loong-cli`、`loong-app-protocol`、`loong-plugin-sdk`、`protocol`、
  `bridge-runtime`；每次只处理一个 owner 明确的 forwarding shell。
- 从 `Cargo.toml` / `cargo metadata --no-deps` 重建真实 crate DAG，同步 `AGENTS.md`、
  `CLAUDE.md`、`ARCHITECTURE.md`、reader-facing docs 和 architecture checks。

**完成线**

- 没有 phase spine、compatibility facade 或 kernel -> author-facing SDK 反向依赖；
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

## 19. 在 benchmark/profile 证明后迁移 Capabilities bitset

**进入条件**

- 先为当前集合表示建立 membership、subset、intersection、child narrowing 与 clone/owned-Cow
  baseline；workspace benchmark 和真实 profile 必须共同证明这些操作是值得优化的 hot path。
- 若 profile 不支持迁移，关闭该任务并保留当前表示，不为“看起来更快”增加复杂度。

**范围**

- 评估已有 bitset crate；除非现有库无法表达固定 capability universe，否则不手写 bitset。
- `Capabilities` 封装 capability 到 bit index 的 exhaustive mapping；不能把未承诺稳定的 enum
  discriminant 当成持久化/wire bit position。
- 保持 capability 名称/list 的序列化 contract；Policy、Action、Context 和 Access 不暴露 mask、
  index、word size 或具体 bitset crate。

**完成线**

- capability gate、subset 和 intersection 语义与迁移前一致；
- `Cow::Borrowed` base path 不分配，child narrowing 只构造一个 owned bitset；
- benchmark/profile 记录迁移前后结果，并证明收益覆盖新增复杂度。

**验证**

```bash
cargo test -p loong-contracts
cargo test -p loong-core policy
cargo test -p loong-kernel policy
cargo test -p loong-app context
cargo bench -p loong-bench -- capabilities
cargo clippy -p loong-contracts -p loong-core -p loong-kernel --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Code TODO 对照

- `TODO(session-owned-context)` -> 步骤 7；
- `TODO(deprecate-no-kernel-live-source)` -> 步骤 9；
- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 10；
- `TODO(tool-audit-owner)` -> 步骤 5；
- `TODO(deprecate-tool-core-envelope)` / `TODO(tool-plane)` /
  `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` -> 步骤 14；
- `TODO(control-plane-action)` -> 步骤 15；
- `TODO(deprecate-legacy-kernel-auth)` / `TODO(deprecate-legacy-policy-error)` -> 步骤 16；
- `TODO(kernel-contract)` -> 步骤 16。
