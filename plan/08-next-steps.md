# plan: 最小提交顺序

本文件只列尚未完成的实施步骤。每个编号项是一个独立提交候选；完成后删除该项，而不是追加
完成日志。长期不变量见 `01` 到 `07`。

## 1. CapabilityContext 改为借用

**范围**

- 修改 `crates/loong-core/src/policy/context.rs`：
  `allowed_capabilities()` 返回 `&BTreeSet<Capability>`。
- 更新 app/spec/kernel test contexts 和 `PolicyEngine::grant` caller；删除只为 capability gate
  产生的 clone。
- 不在这个提交中改 Context owner 或 capability narrowing 语义。

**完成线**

- capability gate 不 clone 整个 set；
- 所有 `ContextFactory::Cx<'_>` 仍满足 `CapabilityContext + Send + Sync`；
- 没有新增 capability proxy/helper。

**验证**

```bash
cargo test -p loong-core policy
cargo test -p loong-kernel policy
cargo check -p loong-core -p loong-kernel -p loong-app -p loong-spec
cargo fmt --all -- --check
git diff --check
```

建议提交：`refactor(core): borrow execution capabilities`

## 2. ActionGrant 保存 allow PolicyReport

**范围**

- 修改 `crates/loong-core/src/policy/grant.rs`，让 `ActionGrantInfo` 保存完整 allow
  `PolicyReport`；不要增加 getter soup，沿用 grant metadata 的直接字段形状。
- 修改 `crates/loong-core/src/policy/engine.rs`，把已经生成的 allow report 移入 grant info，
  不重新 decide、不丢 evaluation。
- 更新 kernel/access/tool plane tests，断言 grant id 与 allow report 同时保留。

**完成线**

- allow/deny 都保留完整 policy report；
- `Granted<A>` 仍不可伪造，report 不成为复制 grant 的入口；
- 不在本提交中设计 audit event transport。

**验证**

```bash
cargo test -p loong-core policy
cargo test -p loong-kernel policy
cargo test -p loong-access
cargo fmt --all -- --check
git diff --check
```

建议提交：`refactor(core): retain policy reports on action grants`

## 3. 用 Turn-scoped Context 彻底替换 AppContext

**范围**

- 在 app runtime boundary 定义 `Context<'a>` 与 `RuntimeContextFactory`。
- Context 从 Session authority、本次 Turn 的 typed options 和 cancellation signal 构造；归一化
  mode/goal、effective caps、tool config、roots 和其它本次执行 view。
- 将旧 `AppContextInner` 字段逐项归属到 Runtime、Session、Turn options、Context derived view
  或 concrete Action；删除没有 consumer 的 plane/tier 和 duplicated request payload。
- `PolicyAny` 改读 `ActionMeta::payload()`，删除
  `KernelInvocationContext::request_parameters()`。
- nested tool invocation 派生同类型 child Context，只缩窄 caps/tool/root view，继承
  mode/goal/cancellation。
- 一次性迁移 production、tests、fixtures、generic instantiation、注释和文档，删除
  `AppContext`、`AppContextInner`、`AppContextFactory`、COW mutation API 和兼容 re-export。
- advisory Session 也构造同一种 Context；删除用“缺少 Context”表达权限的
  `ConversationRuntimeBinding` / `ProviderRuntimeBinding` 分支。

**完成线**

- `ContextFactory::Cx<'a> = Context<'a>` 真正使用 lifetime；
- Runtime/Session 是长期 owner，Context 不含 `Arc<AppContextInner>`；
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

建议提交：`refactor(app): replace app context with turn execution context`

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
- explicit Session/Runtime shutdown 可以向下取消活跃执行；
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

- 保留已经迁移的 read-only modes、simple apply、无 skills bridge apply/rollback；不再在 plan 中
  重做这些路径。
- 将 `apply_selected + apply_skills_plan=true` 依赖的 `skills.install` / `skills.remove`、external
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

## 8. 收敛 action authorization 与 tool execution audit

**范围**

- 让所有 generic action grant path 强制记录 authorization evidence，包括 direct Access 中的
  PolicyEngine grant，不能只覆盖 `Kernel::grant_action` caller。
- allow 使用 `ActionGrantInfo` 的 report + grant id；deny 使用 typed authorization error report；
  token/pack/capability pre-policy failure明确没有 policy evaluations。
- runtime tool invocation wrapper 强制记录 grant 后 completed/failed/input-error/cancelled，带 stable
  path display 和 grant id；concrete tool 不获得 audit API。
- 删除 contracts/kernel 中 tool-specific registry/path/route schema、
  `Kernel::record_tool_invocation` 和 `TODO(tool-audit-owner)`；legacy `PlaneInvoked` 随 legacy
  planes 删除。

**完成线**

- 每个 grant attempt 恰好一条 authorization evidence；每个已消费 grant 恰好一条 execution
  evidence；
- fs/tool/action grant 不存在绕过 audit 的 direct PolicyEngine 路径；
- authorization deny 不重复成为 tool execution deny；
- audit payload 不暴露 ToolSlot 或 concrete registry key type。

**验证**

```bash
cargo test -p loong-kernel audit
cargo test -p loong-access audit
cargo test -p loong-app tool_invocation
cargo test -p loong -- audit
cargo fmt --all -- --check
git diff --check
```

## 9. 删除 legacy kernel authorization surface

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

## 10. 用 descriptor-relative backend 关闭 fs TOCTOU

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

## 11. 删除 runtime transitional spine 并收敛 crates/docs

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

- `TODO(session-owned-context)` -> 步骤 3；
- `TODO(deprecate-no-kernel-live-source)` -> 步骤 5；
- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 6；
- `TODO(deprecate-tool-core-envelope)` / `TODO(tool-plane)` /
  `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` -> 步骤 7；
- `TODO(tool-audit-owner)` -> 步骤 8；
- `TODO(control-plane-action)` / `TODO(deprecate-legacy-kernel-auth)` /
  `TODO(deprecate-legacy-policy-error)` / `TODO(kernel-contract)` -> 步骤 9。
