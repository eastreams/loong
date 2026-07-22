# plan: 最小提交顺序

本文件只列尚未完成的有序迁移目标。编号已被其它文档和 Code TODO 引用，因此删除已完成目标后不
机械重编号。长期不变量只保留在 `01` 到 `07`。

## 当前 active goal：闭合 loong-runtime-owned typed execution domain

### 设计门禁：Runtime / Session / Context 形状未完成

本 active goal 当前停在 owner/control 设计，而不是等待机械实现。`02-runtime-and-crates.md` 中的
`Runtime -> session::Handle -> Invocation<I> -> private Session runner` 仍是先前候选，不是已经接受的
目标 API。实现步骤 2 到 10 只有在以下问题明确后才能重写并开始：

- Session 是 runner 独占 value，还是受控共享 state 加唯一 lifecycle loop；
- 调用面是 typed one-shot `invoke(I)`，还是可持续 feed input/control 并读取 event/status 的 stream；
- Turn、model step 和 recursive Tool/Access Context 是否需要三个不同层级；
- subagent lifecycle、等待、完成通知与 Runtime shutdown 分别由 registry、channel 和 task owner 中的谁
  负责。

OpenAI Codex 当前实现可作为讨论基线：`ThreadManager` 管理 `Arc<CodexThread>`，`CodexThread` 组合
`Arc<Session>` 与 `SessionIo`，Session loop 创建单一 active task；`TurnContext`、`StepContext` 和
`ToolInvocation` 分层，subagent 是共享 tree-scoped `AgentControl` 的独立 Thread/Session。Loong 只借鉴其
lifecycle/context 分层与 channel 交互，不照搬 broad Session authority、Arc/Mutex 传播或 tool 直接访问
Session。

设计闭合前，只能继续提交不预设上述答案的 typed grant/Access/Tool foundation；不得宣称 Runtime /
Session / Context owner cutover 已经可以实施或完成。

### 先前候选目标

破坏性完成 concrete `Runtime -> Session -> Context<'a>` ownership，并把已经成立但尚未完整接入 owner
主干的 typed grant spine 一起验收。goal 结束时，新架构不能再由 app Context、split
Runtime/Session ownership、legacy dispatcher 或旧 `loong-runtime` transitional code 塑形。

```text
loong_runtime::Runtime
  owns Kernel<RuntimeContextFactory> + ToolPlane + all live Sessions
  -> runtime::Handle
  -> session::Handle
  -> invoke(ConversationInvocation)
  -> Invocation<ConversationInvocation>
  -> private Session runner
       -> private ErasedInvocation adapter
       -> private Context<'a>
            -> InvocationImpl::execute
            -> recursive child Context<'a>
            -> ctx.tool(path)?.invoke(payload)
            -> ctx.access()...
            -> PolicyEngine::grant
            -> ActionGrant<A>
            -> Granted<A>
            -> ToolPlane / Access side effect

legacy ingress
  -> typed NotRegistered / explicitly unmigrated non-tool ingress only
  -> isolated legacy owner
```

`loong-runtime` 直接定义 Runtime、Session、Context、RuntimeContextFactory、handles、Invocation、
`InvocationImpl`/private erasure 和最小 live authority domain；app 只负责 config/persistence
materialization、Policy/Tool registration、concrete `ConversationInvocation`、product orchestration 与
legacy ingress selection。`02-runtime-and-crates.md` 是完整 owner contract。

本 goal 明确取代旧的“app-defined Context + generic Runtime shell”约束。继续有效的是 Context 由统一
runtime domain 定义、Policy 只依赖 requirement traits、`ContextFactory` 只有 GAT；不再为了维持旧 crate
位置保留 app Context 或增加 factory method。

### 已有基础与必须修复的偏差

- 保留当前 `PolicyEngine::grant -> ActionGrant<A> -> Granted<A>`、runtime-bound ToolInvocation、typed
  error、private registered dispatch、operation-local Access 和仅 `NotRegistered` fallback 的实现。
- 删除 app `Context`/Clone `Session`、公开 `Context::new`/`rebind_session`、`RuntimeId` 和长期 owner 现场
  拼 Context 的路径；这些是过渡实现，不形成兼容 API。
- narrowed Context 当前可重新构造 root authority，且 `Context::runtime()` 可达 audit/legacy Kernel；
  cutover 必须从类型和 visibility 上同时关闭两条路径。
- detached typed execution 当前从 legacy dispatcher 取得 Runtime；必须改由 Runtime supervisor 拥有
  child Session lifecycle。
- 当前没有一条可编译的 dependency-inversion boundary 让 runtime-owned Session runner 执行 app-owned
  conversation/provider algorithm。不能用整个 `ConversationRuntime` forwarding trait、global program
  factory、裸 closure 或 `Runtime<P>` 填洞；必须建立 concrete `InvocationImpl` + runtime-private
  erasure，并让 app 的一次调用状态由 concrete `ConversationInvocation` owned value 携带。
- crate-root spine 与 projection relocation 不属于剩余工作；后续迁移不得恢复对应 alias、adapter 或
  forwarding dependency。

### 原有原则中继续生效的硬约束

- 唯一 production typed grant API 是现有 `PolicyEngine::grant`。禁止新增 `Kernel::grant`、grant
  forwarding trait/helper 或第二套 policy engine。
- `ActionGrant<A>` 保留完整 report/info；`Granted<A>` 保持 private mint 并作为 execution proof。禁止
  `SessionAuthority`、permit token、authorization wrapper 或“防伪造”同义类型。
- `ContextFactory` 只有 GAT，没有 factory method。concrete `Context<'a>` 是 recursive execution scope，
  不是 Turn、Session、Invocation result 或 host handle。
- Context 不保存 request parameters、tool/action payload、`ExecutionPlane` 或 `PlaneTier`；
  `PolicyAny` 直接读取 `ActionMeta::payload()`，不从 ambient Context 重建 legacy request。
- `ToolPath` 由 contracts 固定为 non-empty opaque segments，canonical text/serde 使用 leading slash；
  runtime ToolPlane 只决定索引结构。provider/discovery 名称是 registration presentation，不能参与
  authority lookup 或被重新解析成 path。
- Context 必须 cheap Clone；effective capabilities 使用 `Cow<'a, Capabilities>`，child authority 只能取
  parent 交集。任何 caller 都不能通过 Session id/Handle/root constructor 恢复 baseline。
- parent/user permission 缺少真实 interaction 时默认返回结构化
  `PermissionRequestError::Unavailable`，不能 panic；grant error 保留原始 report/source。
- typed Tool/Access 不接收 pack/token，不要求 `KernelInvocationContext`，也不把
  `PolicyGrantError`/typed invocation error 映射回 legacy `KernelError` 或字符串。
- Kernel 的 typed policy/access impl 不带 legacy bearer context bound；仅 isolated legacy owner 的具体
  method 可以约束旧 token/pack context，不能把 HRTB 或同义全局 bound 传播回 ordinary Kernel API。
- Context 不暴露 Runtime/Session owner、Kernel、audit sink 或 `ctx.audit()`。ToolImpl 不参与 audit；
  runtime ToolInvocation wrapper 自动、强制记录 grant-bound execution outcome。
- app execution 通过 owned `I: InvocationImpl` 扩展；`session::Handle::invoke(I)` 只在调用点 generic，
  Runtime/Session/Handle 不传播 `I`。private erasure 必须保留 associated event/output/error，禁止
  `Any`/JSON envelope、String error、`InvocationFactory`、global callback bag 或裸 future injection。
- Session runner 在 `I::execute` 外强制 lifecycle、serialization、cancellation、audit 与 terminal
  delivery；concrete `ConversationInvocation` 不获得 audit/Kernel/Runtime/root constructor。现有
  `ConversationRuntime` 只能是 app implementation 内部 contract，不能注册进 runtime。
- migrated side effect only Access can do。Tool、Policy、Kernel adapter 和 orchestration 不直接执行
  filesystem/network/process 等物理副作用。
- typed hit 后任何 input/grant/dispatch/audit error 都不 fallback。legacy bearer/envelope 只能存在于
  明确 allowlist 的最终 ingress owner，且不能反向拥有 typed Runtime/Session。
- 敢于 breaking change：不留 alias、proxy、双 Context、双 Runtime、deprecated wrapper、同构转换或
  compatibility workaround。替代 API 落地的同一 commit 删除旧入口和 caller。
- helper 默认不成立。保留 helper 必须统一多个真实调用面、无法由类型/owner 表达，并有简短 why/owner
  注释。错误优先使用 `thiserror` 保留 source，`anyhow`/String 只在最终聚合边界使用。
- 每个 ownership、安全边界和非显然约束都写少量 intent comment；不写注释墙，也不能依赖本次讨论
  记忆维持正确性。

### 先前候选步骤（设计闭合后重写）

1. **先收拢已成立的 typed foundation**：按 contracts/core/kernel/access/tool-plane/tool owner 审计现有
   改动；删除无用 helper/TODO 后形成可独立编译的最小提交。app Context 的 authority 重建漏洞
   修复前不得把它作为完成形态提交。
2. **迁移最小 runtime domain**：把 concrete `RuntimeContextFactory`、Context、Session identity/lineage、
   ToolView、caps/fs authority、mailbox/lifecycle 和 Context 所需 typed service contracts 移入
   `loong-runtime`。runtime 不依赖 app；不能把 `LoongConfig`、repository、prompt/provider state 或整份
   `ToolRuntimeConfig` 一起搬入。
3. **建立 SessionSpec materialization boundary**：app 从 config/repository/delegate evidence 验证并构造
   runtime-owned `SessionSpec`。Runtime 接收后复查 parent generation/narrowing 并取得 live ownership；
   删除 app executable Session 和同构 mutation/rematerialization API。
4. **实现 Runtime supervisor/Handle**：concrete Runtime 不可 Clone，中央拥有 Kernel、ToolPlane 和全部
   Session join owner；lifetime-free `runtime::Handle` 只含 command endpoint。shutdown 关闭新命令并
   drain Session；不增加 `RuntimeInner`/`*Shared` 大容器。
5. **建立 invocation extension**：在 runtime 定义非 object-safe `InvocationImpl` 与 typed
   `Invocation<I>`，用 private sealed `ErasedInvocation` adapter 把不同 concrete invocation 放入 Session
   mailbox。adapter 保存 typed channels 并在 app code 外执行 lifecycle/cancel/audit/finalization；不增加
   business envelope、factory 或 generic Runtime。
6. **实现 Session owner/Handle/Invocation**：Session value 只由 runner 持有；Runtime supervisor 保存
   id/generation、parent、join、command/status endpoints。ordinary inbox bounded，interrupt/shutdown 使用
   独立 control channel，receiver 保持 runner-private；删除 kernel `AgentMailbox`/外部 `drain`。Handle
   提供 `invoke(I: InvocationImpl)`；Invocation 不可 Clone并拥有本次 typed event/result/cancel endpoint。
   attached/detached 关系只在 Runtime supervisor graph 中改变。
7. **原子切换 Context**：root Context 只在 Session runner 内 private 构造，child 只取 authority 交集；
   删除 app Context/Factory、`Context::new`、`rebind_session`、`RuntimeId` 和 runtime mismatch。
   concrete Context 实现窄 `ToolInvocationContext` requirement；`ctx.tool()`/`ctx.access()` 使用 private
   narrow services，不暴露 owner。
8. **实现 app concrete invocation 并迁移 host**：`ConversationInvocation` owned capture normalized input、
   provider/conversation services 和 product options，只调用一个明确 coordinator entry。借用型
   `TurnExecutionOptions<'_>`/event sink 改为 owned field 或 `Invocation<I>` event stream；daemon/chat/
   channel 长期只保存 runtime/session Handle，不传播 raw `Arc<Runtime>`/Clone Session。subagent 是 child
   Session，detached work 向 Runtime reparent；删除 `legacy_tools.execution_runtime()` 等反向依赖。
9. **收敛 audit 与 legacy ingress**：operational audit 由 Runtime/Session orchestration 的真实 owner
   调用，不再从 Context 获取 Runtime。因为 Runtime 拥有 Kernel，kernel/core target fallback 放入
   `loong-runtime` 内明确的 legacy ingress module；app-only legacy tool 仍由 app ingress dispatch。
   app 只在 typed miss 后做一次 target routing，runtime 不反向依赖 app，也不增加 forwarding callback
   或同构 wrapper。
10. **清理 runtime API/Cargo/tests**：删除浅层 generic `Runtime<C>`、`id()`、`legacy_kernel()`、generic
    `record_audit_event()`、外部 Context 参数式 `Runtime::access/tool`，以及迁移后无 caller 的 errors、
    fixtures、features/dependencies。测试按 runtime/session/context/invocation/tool_plane owner 拆分。
11. **同步 plan/docs/guards**：architecture check 必须检测 root Context 重建、Context -> Runtime/audit、
    typed -> legacy lifecycle 反向依赖和 legacy import allowlist，而不只是检查旧类型名。

### 先前候选提交顺序（设计闭合后重写）

1. typed contracts/policy foundation；
2. Access 与 concrete tool operation ownership；
3. runtime-owned authority types 与 SessionSpec；
4. `InvocationImpl`、typed Invocation handle 与 private erasure；
5. Runtime supervisor 与 Session owner/Handle；
6. concrete Context 原子 cutover；
7. app `ConversationInvocation` 与 host/subagent caller migration；
8. audit/legacy containment 与 helper 清理；
9. docs、architecture guards 和完整验收。

“最小”指 smallest coherent owner change，不以文件数为目标。每个提交必须独立编译、首行后有详细
body；不能把 filesystem、纯文档、机械 caller migration 和无关行为变化揉成一个提交。不得 reset、
stash 或丢弃当前用户改动；使用精确 staging 组装提交。

### 候选方案完成线

以下必须无输出：

```bash
rg -n "AppContext|AppContextInner|AppContextFactory|KernelInvocationContext" crates
rg -n "Context::new|rebind_session|RuntimeId|runtime_id" crates
rg -n "Arc<Runtime|Arc<Session" crates/app crates/daemon crates/loong-runtime
rg -n "ctx\.runtime\(\)|context\.runtime\(\)|session_context\.runtime\(\)" crates
rg -n "legacy_tools\.execution_runtime" crates
rg -n "Kernel::grant|ActionAuthorizationContext|SessionAuthority" crates
rg -n "RuntimeSpine|RuntimeSurface|RuntimeOneshot|RuntimeInteractive" crates/loong-runtime
rg -n "ConversationRuntime|LoongConfig|AgentTurnRequest|TurnExecutionOptions" crates/loong-runtime
rg -n "InvocationFactory|SessionProgram|dyn .*InvocationImpl|Runtime<RuntimeContextFactory>" crates
rg -n "loong_kernel::mailbox|AgentMailbox|InterAgentMessage|trigger_turn" crates/app crates/loong-runtime
```

以下必须满足精确 owner allowlist：

```bash
rg -n "grant_action|CapabilityToken|authorize_operation" crates/app crates/loong-runtime crates/access
rg -n "ToolCoreRequest|ToolCoreOutcome|CoreToolAdapter" crates/loong-runtime crates/access crates/tools
```

- app 不再定义/re-export concrete Runtime/Session/Context/RuntimeContextFactory；`loong-runtime` 不依赖
  `loong-app`；
- narrowed Context 无法重新 mint root authority；root constructor 对 Session runner 外不可见；
- 至少两个不同 concrete `InvocationImpl` 能经同一 Session mailbox 返回各自 typed event/output/error；
  runtime/private erasure 不使用 `Any`、JSON business envelope 或 String error；
- app 的 production turn 由一个 concrete `ConversationInvocation` 进入 Session runner；不存在把
  `ConversationRuntime` 方法逐个 forwarding 到 runtime 的第二套 behavior surface；
- Runtime shutdown 关闭 spawn/reparent、取消并等待全部 live Session；stale generation fail closed；
- detach 不改变 Session identity、history coordinate 或 authority；
- typed Tool/Access 继续经过现有 grant chain，typed tool execution audit 强制发生；
- typed deny/error 不 fallback，NotRegistered fallback 不读取或构造 typed owner；
- `loong-runtime` 没有旧 transitional public API、无 caller helper 或仅为已删代码存在的 dependency；
- 至少一轮 security/ownership subagent review 和一轮 helper/comment/taste review 均无 blocker。

### 验证

```bash
RUSTC_WRAPPER= cargo fmt --all -- --check
RUSTC_WRAPPER= cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTC_WRAPPER= cargo test --workspace
RUSTC_WRAPPER= cargo test --workspace --all-features
./scripts/check_architecture_boundaries.sh
git diff --check
```

### 本 goal 明确不做

- Capabilities bitset；
- 全部 legacy tool/plane 迁移和 bearer surface 最终删除；
- provider/gateway/Access 的完整 streaming cancellation 接线；
- descriptor-relative fs TOCTOU closure；
- provider/model/history ownership 重写；
- generic Access action execution evidence；
- production permission UI 接线；
- Monty/code-mode programmable agent API。

这些非目标已有后续 owner goal；本轮不得借它们保留双 Context、split Session ownership、typed-to-legacy
反向依赖或旧 runtime API。

## 后续独立目标

## 11. 完整迁移 config.import 到 typed Tool + Access

**范围**

- 当前 `config.import` 整体留在 legacy path；direct preflight 与 `FilePolicyExtension` 保护
  `input_path` / `output_path`。不得引入 `*_with_access` 半迁移函数，也不能按 mode 注册一个“部分
  typed”的同名 tool。
- 先补齐 discovery/read、backup/rollback、manifest、staging/copy/archive/extract/index/remove、skills
  install/remove，以及需要的 network/process Access primitives。每个物理副作用最终都消费
  `Granted<ConcreteAction>`；migration orchestration 只组合 typed facts/results。
- concrete `ConfigImportTool` 只 parse 全部 mode、通过 `ctx.access()` 或
  `ctx.tool(...).invoke(...)` 编排，并构造 typed output。nested tool 不是副作用边界，tool-to-tool child
  capabilities 继续取交集。
- Access primitives 可以按 domain owner 分成前置最小提交；但 typed registration、所有 mode 切换、
  legacy route 删除和 policy/preflight 删除必须作为一个 coherent cutover，不能留下双轨。
- cutover 后删除 `FilePolicyExtension`、direct file preflight 对应分支和
  `TODO(config-import-access)` / `TODO(access-migration)`。

**完成线**

- 所有 `config.import` mode 都命中同一个 typed tool；不存在 mode-based fallback 或
  `*_with_access` 平行实现；
- concrete tool、migration orchestration 和 legacy adapter 不直接执行 filesystem/network/process
  副作用；所有 side effect 都由 Granted Access action 执行；
- `config.import` 不再进入 `Kernel::execute_tool_core`，不再需要 `FilePolicyExtension` 或 direct file
  preflight；
- policy deny 走 typed authorization report/audit，tool execution failure 保留 concrete source，均不
  fallback。

**验证**

```bash
cargo test -p loong-app config_import
cargo test -p loong-app skills
cargo test -p loong-access
cargo check -p loong-access -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

按 Access primitive owner 分拆前置提交；最终 tool registration 与 legacy 删除保持一个原子 cutover。

## 12. 编码 hard constraint 与 terminal consent 阶段

**范围**

- pipeline registration 用明确阶段编码 hard constraints 与 terminal consent；两阶段保持 typed
  `Policy` / `PolicyAny` 对称。
- constraint stage 只允许 `Deny` / `Continue`；`Allow`、permission 或会跳过剩余 constraint 的
  `Advance` 产生结构化 misconfiguration denial。
- terminal consent stage 只在全部 hard constraints 通过后运行；不另造 policy engine，也不在
  pipeline 外重跑 policy。
- 把当前 config-driven `ToolMutationConsentPolicy` 迁入 terminal consent stage；同一 tool invocation
  pipeline 的 visibility 等 hard policy 全部迁入 constraint stage，不能靠 bootstrap 注册顺序维持
  安全性。

**完成线**

- permission 只满足 consent，不增加 capabilities、不覆盖 hard deny/root/runtime limit；
- typed 与 any policy 都能显式注册到正确阶段；错误阶段的 decision fail closed；
- production 不存在 constraint stage 之外的 hard policy，也不存在 terminal consent stage 之外的
  permission policy。

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
- 真实 parent/user interaction backend 只在当前 Runtime/Session owner goal 与步骤 12 完成后启用；此前
  已注册 consent policy 继续通过 `PermissionRequestError::Unavailable` fail closed。

**验证**

```bash
cargo test -p loong-app permission
cargo test -p loong-app session
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 14. 让 streaming Invocation 协作取消并保证 finalization

取消属于一次 Invocation 的递归执行作用域，而不是 Policy 或 Action payload。concrete
`Context<'a>` 私有持有 `tokio_util::sync::CancellationToken`：同一 scope 的 `Context::clone()` 共享
token，递归 tool invocation 与每个 batch sibling 分别通过 `child_token()` 得到独立 child。
parent cancel 向下传播；取消一个 child 不得取消 parent 或 sibling。

runtime `ToolInvocation` 按值持有 child Context，从而拥有该执行作用域；`ToolImpl`、Policy 与 Access
只借用 `&Context<'_>`。它们通过窄的只读 observation contract 检查或等待取消，不能取得 raw token
或 cancellation authority。按值持有 Context 不使它成为 `'static`；detached execution 仍由长期 owner
在自己的 future 内构造 Context。

该目标固定按 owner 拆成五个最小提交，不能合成一个大 cancellation commit：

1. **signal + tool scheduling observation**：Invocation execution owner 引入 live cancellation
   signal 并放入 Context；runtime wrapper 在调度 tool 前观察它，sealed grant 入口通过窄 context
   requirement 在 mint 前观察它。cancellation 是 lifecycle gate，不成为 `PolicyDecision`。定向测试
   触发 signal 后断言不再调度 nested tool，也不再请求下一 action grant；本提交必须有可观察行为，
   不能只传播 signal。增加 `ToolFailureKind::Cancelled` 与 `ToolInvocationError::Cancelled`，不能把取消
   降级成 deny 或普通 execution failure。
2. **provider observation**：provider stream read/retry/backoff 在安全点观察同一 signal，drop
   upstream response stream 并协作退出。
3. **Access observation**：long-running search/glob/read-dir 等 Access operation 在各自安全点观察
   同一 signal；不把 cancellation 变成 policy input。
4. **gateway trigger**：SSE receiver close 触发当前 Invocation signal；gateway 保存自己创建的
   task/join owner，不再 detached 地放任完整 Invocation 继续执行。本提交不先引入无 evidence 的
   force abort。
5. **finalization/evidence**：把 cancellable execution 与不可跳过的 finalization 分开；Started 后
   协作退出记录 `ActionExecutionEvent::Cancelled`，Started 前取消不伪造 execution evidence。同时
   记录 timeout、partial-output policy 和 Session lifecycle transition；等待 grace period 后才允许
   force abort，并记录 timeout。partial text 不写成 completed reply。

已经进入 backend 的 side effect 不承诺回滚。步骤 14 只记录其已有 owner 能证明的 tool/Invocation
outcome；generic Access action execution evidence 在步骤 21 完成前不能假定存在。步骤 14 复用 active
goal 已建立的 Session runner/supervisor，不能另造平行 actor registry 或 Context-owned task supervisor。

**完成线**

- downstream disconnect 停止 provider stream，并且不启动后续 tool/action；
- 当前 Invocation cancelled 后 Session 可以继续下一次 Invocation；
- forced abort 只发生在 grace timeout 后，并与正常 cooperative cancellation 区分；
- 首个提交的测试证明 signal 会阻止后续 tool scheduling 与下一 action grant；
- batch parent cancellation 会取消全部 child，但单个 child deny/cancel 不影响 sibling；
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

- 步骤 17 只清理仍有 caller 的 legacy authorization；不得引入 `Kernel::grant_action` 或同义 wrapper。
- 在步骤 15、16 的 caller 全部清空后，删除 `authorize_operation` 和 legacy
  `PolicyError` conversion。
- 删除不再有 ingress owner 的 token/pack authorization methods 与 bearer evidence type；源码已无
  `KernelInvocationContext`，不得用另一名称恢复同类全局 bound。若某个 wire contract 仍有真实 caller，
  先把该 caller 纳入步骤 15 或 16，不能留 fallback/workaround。
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

- 本步骤只在 active Runtime owner goal 完成后开始；届时检查后续 crate 变更没有恢复
  one-shot/interactive dependency、compat alias、app Runtime facade 或 forwarding adapter，不重复进行
  Runtime cutover。
- 继续逐个审计 `loong-cli`、`loong-app-protocol`、`loong-plugin-sdk`、`protocol`、
  `bridge-runtime`；每次只处理一个 owner 明确的 forwarding shell。
- 从 `Cargo.toml` / `cargo metadata --no-deps` 重建真实 crate DAG，同步 `AGENTS.md`、
  `CLAUDE.md`、`ARCHITECTURE.md`、reader-facing docs 和 architecture checks。

**完成线**

- 没有剩余 forwarding shell、compatibility facade 或 kernel -> author-facing SDK 反向依赖；
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
- `Capabilities` 在 contracts 内封装 capability 到 bit index 的 exhaustive mapping 和唯一的
  `ALL_CAPABILITIES` 遍历顺序；不能把未承诺稳定的 enum discriminant 当成持久化/wire bit position。
  新增 capability 必须同时更新 mapping 与遍历表，并由 compile-time capacity assertion 防止超过 64 bit。
- 保持 capability 名称/list 的序列化 contract；Policy、Action、Context 和 Access 不暴露 mask、
  index 或 word size，raw mask 不进入 serde、audit 或其它 wire format。未来内部存储超过单个 word 时可以
  在不修改这些 contract 的前提下替换。
- 删除 `Cow<Capabilities>` 以及仅为 set-backed clone 成本存在的借用分支；recursive Context 直接保存
  `Capabilities` 值，`PolicyContext::allowed_capabilities()` 按值返回，child 通过位与得到严格不扩权的新值。
- benchmark/profile 可以记录迁移收益，但不再作为修复当前表示的进入条件；本目标首先消除错误的
  ownership/集合建模。

**完成线**

- capability gate、subset 和 intersection 语义与迁移前一致；
- base/child Context 都不因 capability clone 或 narrowing 分配；
- `rg -n "Option<Arc<BTreeSet<Capability>>>|Cow<'[^']*, Capabilities>" crates` 无 production 命中；
- serde roundtrip 继续输出有序 capability 名称数组，且不存在 raw integer mask 的 wire 表示；
- tests 覆盖空集、全集、重复输入、subset、intersection、difference、iteration order 与 64-bit capacity
  assertion。

**验证**

```bash
cargo test -p loong-contracts
cargo test -p loong-core policy
cargo test -p loong-kernel policy
cargo test -p loong-app context
cargo clippy -p loong-contracts -p loong-core -p loong-kernel --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## 21. 设计并闭合 generic Granted Action execution evidence

该目标独立于当前 active goal。sealed grant algorithm 已保证所有 typed Action 的 authorization
evidence，runtime ToolInvocation 已保证自身 execution evidence；不能借二者声称 Access execution
audit 已完成。

**范围**

- 先确定 generic `Granted<Action>` execution evidence 的唯一 owner 与调用签名，使 filesystem 与
  其它 Access action 在真正执行时记录 started/completed/failed/cancelled。correlation carrier 已固定：
  `Granted<A>` 携带同一次 private mint 的 id/info/action，execution owner 不再保留 outer
  `ActionGrant` 或复制 id。
- 需要决定的是 domain-neutral execution event/state machine 的 owner：由 `Granted::run` 的 owning port、
  每个 Access operation wrapper，或另一个真正拥有 action outcome 的单一边界强制 Started/terminal
  顺序。不能让每个 caller 自愿记 audit，也不能让 Kernel 代替 domain owner 执行 action。
- `Granted` 携带 immutable authorization info 只是 correlation proof；仍不把 sink、clock 或 authority
  handle 挂到 Context/`Granted`，不增加
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

- generic execution evidence 的唯一 owner/调用签名和 domain-neutral state machine 已经确定；
- 每个 migrated Access action 的 authorization 与 execution event 使用 `Granted.id()` 关联同一 grant，
  且没有 outer-id copy 或第二套 metadata path；
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

## 22. Monty 接入后暴露 governed programmable agent control

这是 deferred goal，不属于当前 Runtime owner cutover。当前 goal 只建立 Session runner、supervisor、
mailbox ownership 与 typed Invocation 基础；在 Monty 真正接入前，不预先冻结 `busy`、`send_msg`、
`redirect` 等 author-facing API。

**前置条件**

- 当前 Runtime/Session/Context active goal 已完成，agent lifecycle、generation、parent relation、history、
  cancellation 与 mailbox 都已有唯一 owner；
- Monty 已作为 agent-side programmable tool 接入 typed ToolPlane，并能把 host dataclass method call 转成
  external future；不能直接照搬当前 MVP 中拒绝 method call、未接通 `ResolveFutures` 的 bridge；
- agent control 使用现有 Context authority、typed policy/grant 和 runtime audit，不建立另一套 sender/token
  授权面。

**已经确定的边界**

- Monty 是 agent 使用的可编程工具，不是 Spec、global metadata 或 Session store 的替代品。
- host 暴露的 `Subagent` 是 frozen、serializable reference，只保存 `agent_id + generation`。它不是
  `Session`、`Arc`、mailbox sender、authorization proof 或 lifecycle owner。
- 每次 method call 都经当前 recursive Context 进入 Runtime supervisor，由 supervisor 校验 reference、
  lineage、effective capabilities 和 operation policy；Monty object 不能直接访问 mailbox 或 registry。
- Runtime supervisor 持有 agent lifecycle record、join、parent relation 与 command/status endpoints；唯一
  Session runner 持有 Session value、mailbox receiver、history 和当前 Invocation。普通 command 使用
  bounded channel，interrupt/shutdown 使用独立 control channel，状态与完成通过 observation subscription
  暴露。
- live status query 返回 immutable snapshot。观察结果不能作为随后 mutation 的正确性前提；需要
  “检查后执行”的语义时，由 Runtime 提供单个 atomic operation。`interrupt` 必须幂等。
- parent 与 child 可以并行；parent 通过 status subscription 观察，通过 Runtime wait operation 等待。
  child authority 只能继承或收窄，任何 Monty 调用都不能借 agent reference 扩权。
- concrete Monty 实现优先使用其已有 host dataclass methods、external futures 与 `asyncio.gather`；没有
  证据时不另造 IR、手写协程状态机或把 live Rust handle 序列化进解释器状态。

**接入前必须决策的问题**

- `send_msg` 是同步 tell，还是等待 Runtime 接受/持久化后完成的 async operation；
- idle agent 收到 message 后是排队、自动启动新 Invocation，还是要求显式 follow-up；
- `redirect` 是否作为独立 atomic operation，以及它与 `interrupt + send_msg/followup` primitives 的关系；
- `status` / `busy` 的最终 async surface 与 snapshot freshness contract；
- Monty REPL state 是否跨 Runtime restart 持久化，以及 stale `generation` reference 的恢复方式；
- `spawn_agent`、`send_message`、`followup_task`、`wait_agent`、`interrupt_agent`、`list_agents` 的最终
  author-facing 命名和 payload/result schema。

这些问题必须在 Monty bridge 与真实 caller 同时可验证时决定，不能先用 helper、alias 或兼容 wrapper
填满 API。无论最终命名如何，状态 mutation 都必须由 Runtime supervisor 串行化；`busy/status` 只用于
观察，不允许形成 check-then-act correctness contract。

**完成线**

- Monty code 可以创建/引用 child agent、并行工作、观察、等待、发送消息和幂等中断，且所有调用都经过
  current Context 的 authority narrowing、typed grant 与强制 audit；
- stale generation、越权 child caps、已关闭 Runtime/Session 和 full mailbox 都返回 typed error；不
  panic，不暴露 sender/receiver，不把 transport 字符串当 policy decision；
- parent completion/cancellation、detach/reparent 和 Runtime shutdown 与 active goal 的 supervisor graph
  保持同一套 lifecycle 语义；
- tests 覆盖 parent/child 并行、wait/status observation、stale reference、capability non-escalation、
  interrupt idempotence、mailbox backpressure，以及最终选定的 idle/send/redirect 语义；
- plan、Monty host-object docs 与 intent comments 明确区分 reference、handle、Session owner、Invocation 和
  immutable status snapshot。

**验证方向**

```bash
cargo test -p loong-runtime agent
cargo test -p loong-tools monty
cargo test -p loong-app subagent
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Code TODO 对照

这些 tag 只是当前代码位置索引，不继承注释中“先加 `#[deprecated]`”的旧指令。步骤 15/17 在 caller
清零后直接删除 legacy API 与 TODO，不建立 deprecation 兼容期。

- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 11；
- `TODO(deprecate-tool-core-envelope)` / `TODO(legacy-tool-core)` /
  `TODO(legacy-tool-view)` / `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` -> 步骤 15；
- `TODO(control-plane-action)` -> 步骤 16；
- `TODO(deprecate-legacy-kernel-auth)` / `TODO(deprecate-legacy-kernel-envelopes)` -> 步骤 17；
- `TODO(kernel-contract)` -> 步骤 17。
