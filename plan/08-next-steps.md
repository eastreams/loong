# plan: 最小提交顺序

本文件只列尚未完成的有序迁移目标。每个目标按 owner 和可独立验证的边界拆成最小提交；除
明确标注为原子替换的步骤外，不把一个编号机械塞进单个 commit。目标完成后删除该项，而不是
追加完成日志。长期不变量见 `01` 到 `07`。

## 1. 删除不属于 execution Context 的旧字段

**范围**

- 删除 `KernelInvocationContext::request_parameters()` 与各 concrete/test context 中的副本；
  `PolicyAny` 直接读取 `ActionMeta::payload()`。
- 删除 `AppContext` 的 `plane` / `tier` 字段、getter 和 `for_invocation` 参数；执行域继续由
  `ToolInvocation`、`AccessCx` 和具体 Action 类型表达。
- legacy audit 若仍需 route/tier，调用边界显式传入，不能从 Context 偷渡。

**完成线**

- Context 不再复制 Action payload；
- `rg -n "request_parameters|\.plane\(\)|\.tier\(\)" crates` 不命中旧 context API；
- 不改变 capability narrowing、tool dispatch 或 policy ordering。

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong-app context
cargo test -p loong-app tools
cargo check -p loong-spec -p loong-kernel -p loong-app
cargo fmt --all -- --check
git diff --check
```

建议提交：`refactor(app): remove non-context invocation metadata`

## 2. 消除 outer/root 与 session-specific 双 Context

**范围**

- channel/gateway/turn service 长期只持有共享 Runtime，不在具体 session identity 未知时构造
  host/root `AppContext`。
- session address 确定后再签发或恢复当前 session authority；同一 Turn 的 provider、core tool、
  app tool 和 policy 全部使用这一份 session-specific execution context。
- 修复 core tool 从 `ConversationRuntimeBinding` 取 outer context、而 preflight/app tool 使用
  session-specific context 的分流；删除对应 downstream binding 传播。
- OpenAI gateway 共享 Runtime，停止每个 request 重新 bootstrap kernel/tool plane。

**完成线**

- 一个 Turn 不同时持有 outer/root context 与 session-specific context；
- channel session 的 workspace、narrowing 和 capability authority 能进入 core tool typed path；
- Session 可以先按 Turn 从 repository snapshot 物化，不引入永久 actor 或 registry。

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

建议按 channel/app 与 daemon gateway 两个 owner 边界分别提交，不把两个 bootstrap 迁移塞进一个
commit。

## 3. 用 recursive execution Context 彻底替换 AppContext

**范围**

- 在 app runtime boundary 定义 `Context<'a>` 与 `RuntimeContextFactory`。
- Context 从 Session authority、本次 Turn 的 typed options 和 cancellation signal 构造；归一化
  mode/goal、effective caps、tool config、roots 和其它本次执行 view。
- 定义 owned `Session`，吸收 identity、lineage、token evidence、session mode、durable
  execution source 和其它跨 Turn 状态；repository record/snapshot 只负责持久化投影。
- 将步骤 1 清理后剩余的 `AppContextInner` 字段归属到 Runtime、Session、Turn options 或 Context
  derived view，不保留同一状态的两份 source of truth。
- Turn boundary 构造 base Context；nested tool invocation 派生同类型 child Context，只缩窄
  caps/tool/root view 并继承 mode/goal/cancellation。Context 表达递归执行作用域，不等同于整个
  Turn，也不固定为某一次 invocation。
- batch tool invocation 为每个分支派生 sibling Context，再 join futures；不能共享可变 overlay。
- subagent 创建新 Session；detached agent/task 向 Runtime 请求托管执行，不能把当前 Context
  伪装成长生命周期 owner。
- detached task 只 move Runtime、owned Session 和 Turn options，并在 task 内重建 Context；
  `&Context` 不进入 `'static` task。
- 一次性迁移 production、tests、fixtures、generic instantiation、注释和文档，删除
  `AppContext`、`AppContextInner`、`AppContextFactory`、COW mutation API 和兼容 re-export。
- 同一提交重写 `plan/02-runtime-and-crates.md` 的“当前事实”和 Context 迁移段落；plan 中不能在
  类型已经删除后继续把 `AppContext*` 描述成现状或待办。
- advisory Session 也构造同一种 Context；删除用“缺少 Context”表达权限的
  `ConversationRuntimeBinding` / `ProviderRuntimeBinding` 分支。

**完成线**

- `ContextFactory::Cx<'a> = Context<'a>` 真正使用 lifetime；
- `Context<'a>: Clone` 的 base path 只复制 Runtime/Session 引用与 borrowed `Cow` view；child
  override 只把被收窄的 caps/tool/root 字段替换成 `Cow::Owned`，不预先铺满 `Arc`；
- Runtime 是长期 owner，Session 是 Context 之外的 owned materialization；Context 不含
  `Arc<AppContextInner>`，也不承诺 Session object 跨 Turn 常驻；
- `ctx.access()` 和 `ctx.tool(path)?.invoke(payload)` 是普通执行入口；
- entry surface 不重复签发同一 Session authority；
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

该步骤是一次 workspace-wide type replacement，可以是大提交；不能为了缩小 diff 留 alias 或双
Context 路径。

建议提交：`refactor(app): replace app context with recursive execution context`

## 4. 让 streaming Turn 协作取消并保证 finalization

**范围**

- 为 Context 建立 Turn-scoped cancellation signal；nested execution 继承同一信号。
- 修改 `crates/daemon/src/gateway/openai_compat.rs`：SSE receiver close 触发当前 Turn cancel，
  不再 detached 地继续完整执行。
- 修改 provider streaming request/retry loop，在 stream read 和 backoff 等安全点观察取消；取消时
  drop upstream response stream。
- conversation runner 将 cancellable execution 与必须完成的 finalization 分开；记录 cancelled、
  partial-output handling 和 Session lifecycle transition。
- tool orchestration 在 grant/dispatch 新 action 前检查取消；长时 Access operation 在安全迭代点
  协作退出。已提交 side effect 不伪装回滚。
- 配置 grace period；超时才 force abort，并记录 cancellation timeout。

**完成线**

- downstream disconnect 停止 provider stream，并且不启动后续 tool/action；
- 当前 Turn cancelled 后 Session 可以继续下一 Turn；
- partial assistant text 不写成 completed reply；
- 当前 SSE owner 能取消并等待其创建的 Turn；跨入口按 session/runtime id 取消留给未来真实的
  active-task owner，不能由 Context 或 cancellation token 单独承诺；
- 无永久 Session actor 或仅为 cancellation 引入的 registry/helper 层。

**验证**

```bash
cargo test -p loong-app streaming
cargo test -p loong-app cancellation
cargo test -p loong -- openai_compat_stream
cargo fmt --all -- --check
git diff --check
```

建议提交：`feat(runtime): cancel disconnected streaming turns`

## 5. 将 provider/runtime-self live source 完全迁入 Access

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

## 6. 迁移 config.import skills lifecycle 副作用

**范围**

- 范围严格限于 `apply_selected + apply_skills_plan=true` 依赖的 `skills.install` /
  `skills.remove`、external
  manifest、staging/copy/archive/extract/index/remove 和 failure rollback 拆成 typed tool/access
  boundaries。
- concrete config import tool 只 parse/compose；filesystem side effect 进入
  `ctx.access()`，skills lifecycle tool 调用进入 `ctx.tool(...).invoke(...)`。
- 迁移完成后删除 `FilePolicyExtension`、direct file preflight 对应分支和
  `TODO(config-import-access)` / `TODO(access-migration)`。

**完成线**

- kernel-routed skills apply 不再 fail closed，也不调用 direct filesystem helper；
- legacy direct API 不再需要 `FilePolicyExtension`；
- side effect 只发生在 granted Access action 或另一个受治理 typed tool 内。

**验证**

```bash
cargo test -p loong-app config_import
cargo test -p loong-app skills
cargo test -p loong-access
cargo check -p loong-access -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

建议提交按 primitive/tool 分拆，不把完整 skills lifecycle 塞进一个提交。

## 7. 删除 typed caller 上的 legacy ToolCore ingress

**范围**

- 持有 Context 的 caller 直接调用 `ctx.tool(path)?.invoke(payload).await`；删除
  `execute_kernel_tool_request` 中只搬运 typed payload/outcome 的 bridge。
- descriptor 由 concrete tool/registration 提供；plane list 时组合 path + descriptor。删除 static
  catalog 重复 metadata、legacy display alias 和手写 typed dispatch match。
- 剩余 tools 逐个迁入 typed plane；每个工具单独提交并把 side effect 迁入 Access。
- caller 清空后删除 `ToolCoreRequest` / `ToolCoreOutcome`、`Kernel::execute_tool_core`、
  `LegacyToolPlane`、adapter traits 和 TODO：`deprecate-tool-core-envelope`、`tool-plane`、
  `tool-plane-display`、`tool-catalog-owner`。

**完成线**

- 新增 builtin tool 只需要 concrete `ToolImpl` 和一条 registration；
- typed dispatch、prompt catalog、search metadata 不再修改全局 match/helper；
- `rg -n "execute_tool_core|ToolCoreRequest|ToolCoreOutcome"` 只允许命中明确保留的非-tool
  domain type；legacy tool envelope 无 production caller。

**验证**

```bash
cargo test -p loong-runtime tool_plane
cargo test -p loong-app tools
cargo test -p loong-app conversation
cargo check -p loong-contracts -p loong-core -p loong-runtime -p loong-kernel -p loong-app -p loong
cargo fmt --all -- --check
git diff --check
```

## 8. 让 direct Access 使用 Kernel generic grant

**范围**

- 在 `loong-core` 定义窄 `ActionAuthorizationContext`，只从统一 Context 借出当前 Session 的
  `CapabilityToken`；effective capabilities 继续由 `PolicyContext` 提供。不要重新引入
  `KernelInvocationContext` 或让 Access 接收散装 pack/token 参数。
- 将 `loong_core::kernel::Kernel<C>` 的跨 crate contract 从 `policy_engine()` 破坏性替换为 async
  generic grant。concrete Kernel 从 `token.pack_id` 查自己的 pack registry、从自己的 clock 取时间，
  并在 permission await 前后验证 token expiry/revocation、pack 与 effective capabilities。
- generic grant 返回 core-owned typed `AuthorizationError`，覆盖 pack/token/capability/policy/audit
  failure，不暴露 concrete `KernelError`。同步收敛现有重复/legacy authorization variants；
  `FsAccessError` 用 `thiserror` source 透明承载，不增加手写 `From` / mapping helper。
- `FsAccess` 持有实现该 core contract 的 Kernel 引用并迁移全部 action grant；access crate 不反向
  依赖 concrete `loong-kernel`，也不继续拿裸 `PolicyEngine`。

**完成线**

- `rg -n "policy_engine\.grant|policy_engine\(\)" crates/access crates/kernel/src/access.rs` 无输出；
- direct Access 与 tool action 使用同一个 token-aware Kernel grant contract；
- Context 的 token 来源唯一是 Session authority，child invocation 只能缩窄 effective caps；
- permission await 后 direct Access 复查 token expiry/revocation 与 effective capabilities。
- access caller 能保留 typed authorization source，不接触 `KernelError` 或字符串降级。

**验证**

```bash
cargo test -p loong-kernel permission
cargo test -p loong-kernel access
cargo test -p loong-access
cargo test -p loong-access permission_token_recheck
cargo check -p loong-core -p loong-access -p loong-kernel
cargo fmt --all -- --check
git diff --check
```

## 9. 编码 hard constraint 与 terminal consent 阶段

**范围**

- pipeline registration 用明确阶段编码 hard constraints 与 terminal consent；两阶段都保持 typed
  `Policy` / `PolicyAny` 对称，不把 action-specific permission 逼成 untyped fallback。
- constraint stage 只允许 `Deny` / `Continue`；`Allow`、permission 或会跳过剩余 constraint 的
  `Advance` 必须产生结构化 misconfiguration denial。
- terminal consent stage 在所有 hard constraints 通过后运行，并保留 `Allow` / `Deny` /
  permission 与 `Continue` / `Advance` 的既有短路语义。不要另造第二套 policy engine 或靠 helper
  在 pipeline 外重跑 policy。

**完成线**

- permission 只满足 consent，不增加 capabilities、不覆盖 hard deny/root/runtime limit；
- typed 与 any policy 都能显式注册到正确阶段；错误阶段的 permission 决策 fail closed；
- constraint stage 的 `Allow` / `Advance` 同样 fail closed，不能跳过后续 hard deny；
- production 尚未接线前不注册 permission policy。

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 10. 记录 attempt-correlated authorization audit

**范围**

- Kernel 在 policy evaluation 前分配只用于 audit correlation 的 authorization attempt id；这不是
  permit/grant token，不能用于执行 action。
- 每个已结束 attempt 记录恰好一条 terminal authorization event。permission requested、resolved、
  escalated 或 request failed 是零到多条 interaction event，并与 terminal event 共用 attempt id。
- allow terminal event 使用 `ActionGrantInfo` 的 report + attempt id + grant id；deny 使用 typed
  authorization error report + attempt id。token/pack/capability pre-policy failure 明确没有 policy
  evaluations；request failure 与 denial 保留原始 `PolicyReport`。
- grant boundary 自动记录这些 evidence；concrete policy、tool 与 Access backend 不获得裸 audit
  API，也不手写 authorization event。

**完成线**

- completed/denied/failed grant attempt 各有且仅有一条 terminal authorization event；
- permission interaction 可独立审计，并与最终 outcome 保持同一 attempt/action/report 关联；
- fs/tool/action grant 不存在绕过 audit 的 direct PolicyEngine 路径。

**验证**

```bash
cargo test -p loong-kernel audit
cargo test -p loong-kernel permission
cargo test -p loong-access audit
cargo fmt --all -- --check
git diff --check
```

## 11. 接通 production parent/user permission interaction

**范围**

- 在 production Context 实现 `request_parent_permission` / `request_user_permission`：parent 是
  当前 Session 的 parent，user 是 Session 树之外的 root actor。
- parent 可以升级给 user；user 必须 terminal。interaction unavailable 返回结构化错误，不能
  落到 `PolicyContext` 的默认 panic。
- Runtime/Session orchestration 负责把请求送到真实 interaction surface 并返回 resolution；
  Context 只提供当前递归执行作用域，不保存 UI/provider handle。

**完成线**

- production permission 路径不会触发默认 panic；
- parent/user 路由与 Session lineage 一致，user escalation 被结构化拒绝；
- production 只在步骤 8 到 10 的 token-aware grant、hard-gate ordering 与 audit 完成后注册
  permission policy。

**验证**

```bash
cargo test -p loong-app permission
cargo test -p loong-app session
cargo test -p loong-kernel permission
cargo fmt --all -- --check
git diff --check
```

## 12. 收敛 tool execution audit

**范围**

- runtime tool invocation wrapper 强制记录 grant 后 completed/failed/input-error/cancelled，带 stable
  path display 和 grant id；concrete tool 不获得 audit API。
- 删除 contracts/kernel 中 tool-specific registry/path/route schema、
  `Kernel::record_tool_invocation` 和 `TODO(tool-audit-owner)`；legacy `PlaneInvoked` 随 legacy
  planes 删除。

**完成线**

- 每个已消费 tool grant 恰好一条 execution evidence；
- authorization deny 不重复成为 tool execution deny；
- audit payload 不暴露 ToolSlot 或 concrete registry key type。

**验证**

```bash
cargo test -p loong-app tool_invocation
cargo test -p loong -- audit
cargo fmt --all -- --check
git diff --check
```

## 13. 删除 legacy kernel authorization surface

**范围**

- control plane 定义 typed action 并使用 generic grant，删除 explicit legacy allow bootstrap 和
  `TODO(control-plane-action)`。
- caller 清空后删除 `authorize_kernel_action`、`policy_engine_error`、legacy `PolicyError` 转换和
  TODO：`deprecate-legacy-kernel-auth`、`deprecate-legacy-policy-error`。
- 保留现有最小 `loong_core::kernel::Kernel<C>` Access contract；没有真实外部需求时删除
  `TODO(kernel-contract)`，不新增宽 forwarding trait。

**完成线**

- production caller 不再调用 legacy authorization API；
- typed authorization error/report 到 owning boundary 前不降级成字符串或 extension error；
- kernel 不出现第二套同义 governance trait。

**验证**

```bash
cargo test -p loong-kernel policy
cargo test -p loong -- control_plane
cargo check -p loong-core -p loong-kernel -p loong
cargo fmt --all -- --check
git diff --check
```

## 14. 用 descriptor-relative backend 关闭 fs TOCTOU

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

## 15. 删除 runtime transitional spine 并收敛 crates/docs

**范围**

- 删除/迁移 `loong-runtime` crate root 的 `RuntimeSpine`、one-shot/interactive phase API 和无 owner
  re-export，保留 `Runtime<C>` / ToolPlane owner。
- 逐个审计 `loong-cli`、`loong-app-protocol`、`loong-plugin-sdk`、`protocol`、
  `bridge-runtime`；每次只处理一个 owner 明确的 forwarding shell。
- 从 `Cargo.toml` / `cargo metadata --no-deps` 重建 15-crate DAG，同步 `AGENTS.md`、
  `CLAUDE.md`、`ARCHITECTURE.md`、reliability/core-beliefs/single-entry docs 和 architecture checks。

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

## Code TODO 对照

- `TODO(session-owned-context)` -> 步骤 2、3；
- `TODO(deprecate-no-kernel-live-source)` -> 步骤 5；
- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 6；
- `TODO(deprecate-tool-core-envelope)` / `TODO(tool-plane)` /
  `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` -> 步骤 7；
- `TODO(tool-audit-owner)` -> 步骤 12；
- `TODO(control-plane-action)` / `TODO(deprecate-legacy-kernel-auth)` /
  `TODO(deprecate-legacy-policy-error)` -> 步骤 13；
- `TODO(kernel-contract)` -> 步骤 8、13。
