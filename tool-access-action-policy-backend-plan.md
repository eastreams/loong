# Tool Access / Action / Policy / Backend 临时计划

这是临时设计计划，不并入 `docs/`。目标是把当前关于
`loong-core::policy`、tools、access facade、action 派生和 backend 可见性的
讨论整理成后续实现可执行的边界说明。

## 核心思想

tool 的副作用执行只能经由 kernel 持有和组装的 access facade：

```text
tool-facing caller
  -> ToolCx::access(...)
  -> AccessCx<'_, K>
  -> HasFsAccess::fs(...)
  -> FsAccess<'_, K>::read_file(...)
      -> CanonicalPath::resolve(...)
      -> FsReadAction::new(CanonicalPath)
      -> PolicyEngine::grant(...)
      -> ActionGrant<FsReadAction>
```

`Access`、`Action`、`Policy` 是治理语义，可以由 kernel 间接暴露给其它层。
`Backend` 层应拆成 backend trait 和 concrete backend impl：trait 是 kernel-owned
的执行接口，impl 是几种互斥的具体实现之一。backend 选择应通过编译期泛型 /
associated type 固定，而不是运行时 registry 式切换。tools/app/runtime 不把 backend
作为直接依赖面。

这样 tools/app/runtime 只能通过 `ToolCx -> AccessCx -> domain Access` 表达意图，不能绕过
policy 直接调用 backend，也不能自己拼出授权链。各类 policy/domain context 只作为
kernel 内部实现细节存在，不暴露给 tool-facing API。

## 当前 policy 基础

`loong-core::policy` 已经有主要 building blocks：

- `Action`：policy-facing 的副作用意图。
- `PolicyEngine::grant`：授权 action，返回 `ActionGrant<A>`。
- `Granted<A>`：构造函数在 core 内部，不可被外部伪造。
- `Granted<A>::into_action()`：把已授权 action 移交给执行边界。

当前仍有旧模型残留：

- `ActionExecutor`
- `Granted::execute_with`
- `HasPolicyEngine::execute_granted`

目标模型不是再加一层 executor wrapper，而是让 backend/store 方法直接消费
`Granted<ConcreteAction>`，并在内部调用 `into_action()`。

## 边界规则

### ToolCx, AccessCx, and Access

`Access` 是 facade，不是 backend。

工具侧拿到的不是 kernel，也不是 domain context，而是 `ToolCx`。`ToolCx` 最多暴露
一个构造 `AccessCx` 的入口。`AccessCx` 是 kernel-owned access construction context，
内部持有 `&K` 和本次调用已经构造好的统一 `policy_context`。

domain access trait 应该实现在 `AccessCx` 上，表示这个 access context 能给出某个
domain access。当前 fs-read slice 的具体形状是 `HasFsAccess::fs(self) -> FsAccess`。
tool 代码只要求自己的 tool context 能给出
`AccessCx`，以及这个 `AccessCx` 能给出需要的 access。tool 不应该看见
`policy_context`、`WorkspacePolicyContext`、backend handle，或 kernel concrete
fields。

`AccessCx` 的构造是否应该直接 port 到 kernel 现有生成 context 的方法还需要确认。
无论最终是 `ToolCx::access_cx()` 直接委托 kernel，还是 `ToolCx` 持有已构造好的
access context，都应该只有一条受控构造路径。

domain access 方法负责：

1. 把调用方输入验证/规范化成 concrete action；
2. 调用 policy grant；
3. 返回 `ActionGrant<ConcreteAction>`，让后续执行边界只处理已授权 action。

示意结构：

```rust
pub trait ToolCx {
    type Kernel: HasPolicyEngine;

    #[inline(always)]
    fn access(&self) -> AccessCx<'_, Self::Kernel>;
}

pub struct AccessCx<'a, K>
where
    K: HasPolicyEngine,
{
    kernel: &'a K,
    policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
}

pub trait HasFsAccess<'a, K>
where
    K: HasPolicyEngine,
{
    #[inline(always)]
    fn fs(self) -> FsAccess<'a, K>;
}

impl<'a, K> HasFsAccess<'a, K> for AccessCx<'a, K>
where
    K: HasPolicyEngine,
{
    #[inline(always)]
    fn fs(self) -> FsAccess<'a, K> {
        FsAccess::new(self.kernel, self.policy_context)
    }
}

pub struct FsAccess<'a, K>
where
    K: HasPolicyEngine,
{
    kernel: &'a K,
    policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
}

impl<'a, K> FsAccess<'a, K>
where
    K: HasPolicyEngine,
    <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>: WorkspacePolicyContext,
{
    pub async fn read_file(
        self,
        path: impl AsRef<Path>,
    ) -> Result<ActionGrant<FsReadAction>, FsAccessError> {
        let path = CanonicalPath::resolve(path, self.policy_context.workspace_root())?;
        let action = FsReadAction::new(path);
        let grant = self
            .kernel
            .policy_engine()
            .grant(&self.policy_context, action)
            .await?;

        Ok(grant)
    }
}
```

这里不需要 `HasPolicyContext`，也不需要工具直接接触 context。`AccessCx` 携带
`policy_context` 字段；`FsAccess` 从 `AccessCx` 构造出来并私有持有
`policy_context`。该 context 类型由
`HasPolicyEngine::PolicyEngine<'a>::Factory` 推导，并额外实现 domain 需要的
`WorkspacePolicyContext`。

`FsAccess` 的泛型参数是 kernel，不是 backend。backend 类型通过 kernel 的
associated type 决定。policy context 是统一的，domain-specific view 通过 trait
on context 提取。由于 `Action` 是 `'static`，action 构造函数只能用提取出的 view
做校验和派生 owned action data，不能把 borrowed context view 存进 action。

### Action

`Action` 是 policy 和 audit 能看见的 typed intent。

action 构造函数只做 shape validation / normalization，不执行副作用。action
应该携带 policy、audit、backend 执行所需的结构化信息，避免 backend 再解析原始
tool JSON。

当前 `Action` trait 上还有 `execution_plane()` / `plane_tier()`。这个设计需要重新
评估：如果 `ExecutionPlane` 是 kernel dispatch/audit 的外层路由信息，它可能不该是
每个 action 自己声明的方法，而应由 `AccessCx` / invocation route / kernel plane
在授权和审计时提供。迁移计划里不要把 `ExecutionPlane` 继续属于 `Action` 当作既定
结论。

### Policy

Policy 是唯一能 mint `Granted<Action>` 的位置。

policy 输入是 typed action 和 policy context；允许时返回不可伪造 grant，拒绝
时给出结构化、可审计的 denial。

context 不应按 domain 拆成 `fs_policy_context()`、`browser_policy_context()` 这类
kernel 方法。kernel 提供一个统一 context；各 domain 定义 trait on context 来表达
自己需要的 view/capability：

```rust
trait WorkspacePolicyContext {
    #[inline(always)]
    fn workspace_root(&self) -> &Path;
}
```

policy impl 也应依赖 `PolicyContextFactory` 和 HRTB，而不是依赖某个 concrete
context：

```rust
#[async_trait]
impl<F> Policy<F, FsAction> for AllowWorkspaceFsPolicy
where
    F: PolicyContextFactory,
    for<'a> F::Context<'a>: WorkspacePolicyContext,
{
    fn name(&self) -> Cow<'static, str> {
        "allow_workspace_fs".into()
    }

    async fn grant(&self, ctx: &F::Context<'_>, action: &FsAction) -> PolicyGrant {
        // use WorkspacePolicyContext methods on ctx
    }
}
```

这样新增 domain 时扩展的是 context 能力，而不是让 kernel trait surface 不断增长。
这些 context trait 是 policy/access 内部约束，不是 tool-facing API。

这些薄构造和 extract 方法可以加 `#[inline(always)]`，包括：

- `ToolCx::access_cx()`
- `HasFsAccess::fs()`
- `WorkspacePolicyContext` 这类 context view getter

不要把 `#[inline(always)]` 扩散到 `Policy::grant`、backend side effect、或 async
主执行逻辑上；它只用于消除 access/context glue 的边界成本。

### Backend

`Backend` 层是唯一真正执行副作用的地方，但它不应该只是一个 concrete type。

每个 side-effecting domain 应定义一个 kernel-owned backend trait。trait 方法直接
接受 `Granted<ConcreteAction>`，这样 kernel 泛型可以通过 associated type 选择某个
具体实现：

```rust
#[async_trait]
trait FsBackend: Send + Sync {
    async fn read(&self, granted: Granted<FsReadAction>) -> Result<FsReadOutput, FsError>;
}

struct LocalFsBackend;

#[async_trait]
impl FsBackend for LocalFsBackend {
    async fn read(&self, granted: Granted<FsReadAction>) -> Result<FsReadOutput, FsError> {
        let action = granted.into_action();
        // filesystem read side effect happens here
    }
}
```

concrete backend impl 是编译期 N 选一的互斥实现，例如 local fs backend、
sandboxed fs backend、mock backend、remote backend。选择点在 kernel 类型 /
builder 类型 / test support 类型上，通过 generic parameter 或 associated type 固定，
不是在每次 tool call 上动态选择。

backend trait 是否 `pub(crate)` 或更公开，取决于 concrete impl 是否需要跨 crate
提供；但即使 trait 需要公开，tool-facing API 也不应暴露 concrete backend handle，
也不应让 tools 直接调用 backend trait 方法。tools 仍然只通过 access facade 表达
意图。

## 依赖结构

目标 crate 关系：

```text
loong-core
  - policy traits and grant token
  - Action, PolicyEngine, Granted

kernel
  - 组装 AccessCx<'_, K>
  - 组装统一 policy context
  - 通过 associated type / generic parameter 固定 backend impl
  - 持有互斥选择后的 concrete backend
  - 把 access 调用路由到 typed policy grant

app / tools / runtime surfaces
  - 接收 ToolCx
  - 通过 ToolCx 构造 AccessCx
  - 通过 domain access trait 从 AccessCx 构造 domain access
  - 通过 domain access methods 表达意图
  - 不直接依赖 backend trait 或 concrete backend types
  - 不直接依赖 kernel concrete type 或 policy/domain context
```

如果 backend impl 为了文件组织或平台差异需要拆分，也要保持同一个调用规则：
kernel 类型选择一个 backend impl；tool-facing code 不能直接构造或调用 backend。

## 从父授权构造子 action

某些操作是更大授权范围内的细化动作。此时子 action 的 `new` 应直接接收
`Granted<SuperAction>`：

```rust
impl BrowserClickAction {
    pub fn new(
        granted: Granted<BrowserSessionAction>,
        link_id: LinkId,
    ) -> Result<Self, BrowserActionError> {
        let session = granted.into_action();
        // validate link_id under the authorized browser session scope
        Ok(Self {
            session_id: session.session_id,
            link_id,
        })
    }
}
```

这比外部 `TryFrom<Granted<SuperAction>>` helper 更强：API 本身表达了“没有父授权
就不能构造子 action”。

由于 `Granted<A>` 被消费，这个模式默认是线性的。如果某个领域需要在一个已授权
scope 内重复派生多个操作，应显式设计 granted lease/session，而不是随意 clone
grant。

## 禁止形态

- tool 解析 JSON 后不生成 typed action 和 policy grant 就直接执行副作用。
- tool-facing API 暴露 kernel、domain policy context、backend trait object 或
  concrete backend type。
- backend trait 方法接受 ungranted action。
- `execute_granted` 之类 helper 成为主执行抽象。
- 语义上依赖父授权的 child action 可以从 ungranted parent action 构造。
- 为每个 domain 在 kernel 上增加 `fs_policy_context()` 这类 context getter。
- 未经重新评估就继续把 `ExecutionPlane` 固定为 `Action` 的职责。

## 实现计划

### 1. 盘点当前 mixed paths

列出所有会执行副作用的 provider-visible / hidden tool，并记录：

- 当前 JSON request type；
- required capabilities；
- policy extension checks；
- 副作用实际位置；
- backend trait candidate；
- concrete backend impl candidate；
- target action type。
- access context candidate, usually `AccessCx<'a, K>`。
- domain access trait candidate, for example `HasFsAccess for AccessCx`。
- whether current `ExecutionPlane` metadata belongs to action, access context,
  invocation route, or audit routing.

重点文件：

- `crates/app/src/tools/tool_identity.rs`
- `crates/kernel/src/kernel.rs`
- `crates/app/src/tools/file.rs`
- `crates/app/src/tools/shell.rs`
- `crates/app/src/tools/http_request.rs`
- `crates/app/src/tools/browser.rs`

### 2. 收敛 core policy API

保留 `Granted<A>::into_action()` 作为主 handoff。

当 call site 不再需要旧 executor seam 后，移除或弃用：

- `ActionExecutor`
- `Granted::execute_with`
- `HasPolicyEngine::execute_granted`

替代方案不是新 wrapper，而是 backend trait 方法直接消费
`Granted<ConcreteAction>`。

### 3. 先做 filesystem vertical slice

filesystem 工具最容易验证 policy 和 side effect 边界，适合作为第一刀。

当前已落地的 fs-read slice 目标类型：

- `FsAccess`
- `FsReadAction`
- `FsAction`
- `CanonicalPath`
- `HasFsAccess`
- unified policy context
- `WorkspacePolicyContext` trait on policy context

`FsAccess` 应写成 kernel-generic struct：

```rust
pub struct AccessCx<'a, K> {
    kernel: &'a K,
    policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
}

pub trait HasFsAccess<'a, K> {
    #[inline(always)]
    fn fs(self) -> FsAccess<'a, K>;
}

pub struct FsAccess<'a, K> {
    kernel: &'a K,
    policy_context: <K::PolicyEngine<'a> as PolicyEngine>::Cx<'a>,
}
```

`ToolCx` 应只暴露构造 `AccessCx` 的入口；`K` 通过 trait bounds 提供 policy engine；
context 类型从 `K::PolicyEngine<'_>::Factory` 推导并作为 `AccessCx`
字段保存：

```rust
trait WorkspacePolicyContext {
    #[inline(always)]
    fn workspace_root(&self) -> &Path;
}

#[async_trait]
impl<F> Policy<F, FsAction> for AllowWorkspaceFsPolicy
where
    F: PolicyContextFactory,
    for<'a> F::Context<'a>: WorkspacePolicyContext,
{
    fn name(&self) -> Cow<'static, str> {
        "allow_workspace_fs".into()
    }

    async fn grant(&self, ctx: &F::Context<'_>, action: &FsAction) -> PolicyGrant {
        // authorize FsAction using WorkspacePolicyContext methods on ctx
    }
}
```

目标流：

```text
file tool request
  -> ToolCx::access_cx(...)
  -> AccessCx<'_, K>::fs(...)
  -> FsAccess<'_, K>::read_file(...)
  -> CanonicalPath::resolve(path, ctx.workspace_root())
  -> FsReadAction::new(CanonicalPath)
  -> policy.grant(ctx, action)
  -> ActionGrant<FsReadAction>
```

完成后，file tool 不再直接拼 authorization 链；真正的 filesystem side effect
边界仍应在后续 backend 迁移中继续收敛到 `Granted<FsReadAction>`。

### 4. 把 domain checks 从 ad hoc JSON parsing 挪到 typed action

capability inference 可以保留粗粒度，但操作级验证应进入 typed action 和 policy
context。

例子：

- path normalization：进入 `CanonicalPath::resolve`；`FsReadAction::new` 只接收
  已解析的 `CanonicalPath`；
- shell command risk：作为 structured action data 进入 grant 前；
- URL normalization：进入 network/http action construction；
- browser session/link validation：进入 parent-to-child action construction。

### 5. 落地 child action constructor pattern

browser 操作适合作为主要验证场景：

```text
BrowserSessionAction
  -> Granted<BrowserSessionAction>
  -> BrowserClickAction::new(granted_session, link_id)
  -> policy.grant(ctx, click_action)
  -> BrowserBackend::click(Granted<BrowserClickAction>)
```

关键规则：凡是 child action 语义上需要 parent authorization，`new` 就接收
`Granted<SuperAction>`。

### 6. 迁移剩余 tool families

filesystem slice 编译和测试通过后，按风险和复杂度迁移：

1. HTTP/web request tools；
2. shell/bash execution；
3. browser session operations；
4. memory/session durable state changes；
5. hidden specialized tools。

每个迁移完成后，应留下一个清晰的 backend trait method，且该函数接受
`Granted<ConcreteAction>`。具体 impl 是编译期互斥选择，但 side-effect contract
不变。

### 7. 清理 legacy execution seams

所有副作用领域都改为 direct granted-action backend trait methods 后，清理旧
executor 兼容层和相关 docs/tests。

搜索并处理：

- `ActionExecutor`
- `execute_with`
- `execute_granted`
- app/tool code 对 backend trait 或 concrete backend 的直接访问
- 接受 ungranted action 的 side-effect function

### 8. 验证

使用本机 `cargo`，不要用会自动安装 toolchain 的 repo helper。

最低验证：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-features
cargo test --workspace --all-features
git diff --check
```

如果移动 crate 依赖，再跑：

```bash
./scripts/check_architecture_boundaries.sh
```

## 后续：优化 Deny 路径的 Agent 提示

把 policy deny 保留为结构化 authorization error，不在 access/app 侧靠字符串或
`is_policy_denial()` 之类 helper 猜测。`PolicyReport` 应沿 grant/access/tool error
路径传到 kernel tool response 边界，由那里统一生成 Agent-facing 提示：说明被哪个
policy 拒绝、拒绝对象是什么、是否应该换路径/请求授权/停止重试。测试重点放在
`file.read` deny：无读取副作用、错误码稳定、提示可行动，且 legacy preflight 不参与。

## 验收标准

- tool-facing code 不能 name、construct 或 invoke backend trait / concrete backend。
- tool-facing code 不能直接 name 或使用 kernel concrete type、policy context、
  domain context trait；只能经由 `ToolCx -> AccessCx -> domain Access`。
- 所有 side-effecting backend trait methods 都接受 `Granted<ConcreteAction>`。
- domain access trait 实现在 `AccessCx` 上，例如当前 fs-read slice 的
  `HasFsAccess::fs(self) -> FsAccess<'a, K>`。
- domain access 是 `FsAccess<'a, K>` 这类 kernel-generic facade，不直接持有 backend。
- backend impl 通过 kernel associated type / generic parameter 编译期 N 选一。
- `Granted<A>` 仍不可被 `loong-core` 外部伪造。
- `AccessCx` construction 由 kernel/tool context boundary 控制，只有一条受控构造路径。
- 依赖父授权的 child action constructor 接收 `Granted<SuperAction>`。
- `ExecutionPlane` 的归属重新评估后再迁移，不默认保留在 `Action` 上。
- 除非明确批准 breaking change，否则保留现有 public root re-exports。
