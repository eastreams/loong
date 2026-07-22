# plan: 跨模块变更约束

本文件是每个最小提交的 hygiene checklist，不记录完成历史。

## Dependency 与 Feature

- 新增外部依赖或内部 crate 依赖时，先在根 `Cargo.toml` 的 `[workspace.dependencies]` 声明；
  叶子 crate 使用 `*.workspace = true`。
- 不在功能提交中顺手清理无关 crate 依赖。crate 合并/删除是独立 ownership 提交。
- feature flag 控制完整 module/tool family。`loong-tools` 的 `file` feature 关闭时，
  `loong_tools::file` 不存在；模块内部不编译 unavailable stub。
- app feature 只映射 concrete tool feature，例如 `tool-file = ["loong-tools/file"]`。

## Concrete Tool Crate

- `crates/tools` 只放 concrete builtin tool、typed input/output 和小范围 response shaping。
- 不定义或 re-export `ToolPlane`、`ToolImpl`、`RegisteredTool`、Policy、Action abstraction、
  `AccessCx` 或 legacy envelope。
- concrete tool 只约束获取 governed access/tool invocation 所需的小 trait；不依赖 runtime concrete
  Context。调用 fs Access 的 tool 显式要求 `FsResolutionContext + FsPathPolicyContext`，不能把它们
  重新合成一个大 `FsAccessContext`。
- concrete tool 可以通过 `ctx.tool()` 编排 nested typed tool，但物理 side effect 只通过
  `ctx.access()` 进入 owning Access boundary。

## Tool Registration Boundary

- `loong-core::tool` 只拥有 concrete implementer 需要的 `ToolImpl` 抽象和稳定 input/error contract。
  `RegisteredTool`、registered descriptor、`ErasedTool` 与 raw erased invoke 属于 `loong-runtime`；没有
  真实 consumer 时不预造 registration time/provenance wrapper。
- app bootstrap 可以把 concrete `ToolImpl` 注册进 runtime registry，但 registry 不返回可直接 invoke 的
  entry。crate 外 metadata query 只返回不带 dispatch capability 的 view。
- private/sealed erasure 只有在 raw constructor/invoke 同样不可从 runtime 外调用时才真正封住治理
  boundary；不能靠 private trait 包住 public `RegisteredTool::invoke`，也不能补 token/proxy 维持该 API。

## Invocation Boundary

- `loong-runtime::InvocationImpl` 是 app concrete invocation 的单方法 execution contract；
  `loong-app::ConversationInvocation` 持有本次调用的 owned input/services/options。不得把现有
  `ConversationRuntime` 方法集整体搬进 runtime，也不得增加 `InvocationFactory` 或 callback bag。
- public `InvocationImpl` 保留 associated event/output/error；runtime-private `ErasedInvocation` 只为
  heterogeneous Session mailbox 擦除 concrete type。擦除层通过 typed channels 把原结果交还
  `Invocation<I>`，不使用 `Any`、JSON `Value`、String error 或同构 request/result helper。
- private runner 在调用 concrete implementation 外统一负责 lifecycle、cancellation、audit 和 terminal
  delivery。app implementation 不接收 audit sink、Kernel/Runtime、Session owner 或 root Context
  constructor；普通 tool/access 仍只获得 recursive Context。
- invocation value 必须 `Send + 'static`，因为它跨 lifetime-free Session command endpoint。现有借用型
  event sink/options 必须迁成 owned data 或由 `Invocation<I>` 的 event stream 表达，不能通过泄漏、全局表
  或重新 `Arc<Context>` 绕过。

## Access Module Ownership

- 每个 Access domain module 是自己的 public facade，显式 re-export 该 domain 的 public members；
  不把 domain API flatten 到 crate root，也不让 caller 依赖 private operation module path。
- concrete action、options、output、`Access` method 和 granted execution 按 operation 共置。拆大
  `action.rs` 时不能建立另一个集中 action type 的 `action/` 目录；那只改变文件大小，没有修正
  ownership。
- facade/access 文件只保留 identity、construction 和跨 operation 必须共享的强制 chain。单个
  operation 的 side effect、payload shaping 或 policy facts 不能通过 forwarding helper 留在 facade。

## Grant 与 Payload

- `ActionGrant<A>` / `Granted<A>` 的 mint 保持 core-private；执行入口消费 grant。
- `ActionGrant<A>` 只包装一个 `Granted<A>`；id/info/action 全部由同一次 private mint 绑定在
  `Granted<A>` 内，避免 caller 把真实 action proof 与另一份 grant metadata 重组。
- `Granted<A>::id()` / `info()` / `as_ref()` 只用于 execution boundary 的 correlation、authorization
  snapshot 与 action inspection，不能扩成复制、替换 metadata 或绕过 execution port 的 API。
- `ActionMeta::payload(&self) -> Cow<'_, Value>` 没有默认值。已有 JSON 返回 borrowed view，
  按次构造的 structured view 返回 owned value。
- legacy `ToolCoreOutcome` conversion 只存在于明确 allowlist 的 legacy ingress owner；不能放进
  concrete tool、runtime-private ErasedTool 或 Access。

## Helper

保留 helper 前必须同时满足：

1. 至少统一多个真实重复调用面；
2. 该约束不适合由类型、trait bound、owned struct、Action、Context 或 Granted typestate 表达；
3. 附近注释说明为什么需要 helper、为什么属于当前模块、是否为迁移期边界。

只搬运同构数据、包装 `from`/`into`、拼 legacy display alias、隐藏 policy/access construction、
或把应删除 boundary 延长一层的 helper 直接删除。

`ToolInvocationContext` 不是搬运 helper：它是 generic `ToolInvocation<C>` 对 `C::Cx<'a>` 的窄
requirement，只允许派生 authority 不扩大的同类型 child。它不能获得 root constructor、Kernel、audit、
Runtime handle 或其它 Context convenience；也不能再增加第二个同义 child-derivation trait。

Kernel generic operational recorder 不是 forwarding helper：它集中拥有 clock、event id 分配和 sink
write，并将 error boundary 收敛为 typed `AuditError`。但它必须拒绝 engine-owned `Authorization`、
grant-bound `ActionExecution` 与只读历史 `ToolInvocation`；runtime execution 使用绑定真实
`Granted<ToolInvocationAction>` 的窄 recorder，不能让 caller 传任意 grant id/event。runtime 仍不能
读取 kernel-private state，也不能增加
`AuthorizedToolInvocation`、`AuditHandle`、route/receipt 或 `ctx.audit`。

## Error

- 每个 Access domain 拥有自己的 typed error，并用 `thiserror` 保留 source chain。
- 不为单个 `?` 新增无意义 `From`；只有跨多个调用点稳定表达同一层 error boundary 时才实现
  conversion。
- policy denial 通过 typed authorization error/report 传播，不靠字符串分类 helper。
- lookup 与 invoke 是两个 fallible 阶段：`Context::tool` 返回 `LookupError`；
  `with_capabilities_override` 只保存 requested value；`invoke` 返回 runtime-owned
  `ToolInvocationError`。后者覆盖 override rejection、child narrowing、grant、registered tool
  execution 与 execution audit，并保留 `CapabilityOverrideError`、`CapabilityNarrowingError`、
  `PolicyGrantError`、`RegisteredToolError` 和 `AuditError` source。首次 lookup 已绑定 concrete entry
  后，不保留只包一层 tool error 的 `DispatchError`，也不构造 post-grant registry invariant。临时
  typed-first app ingress 可以用一个明确的 sum 组合 lookup/invocation 与 legacy failure，但不能把它们
  压成 `KernelError` 或字符串再分类。
- legacy string envelope 的最后转换只发生在 legacy owner，并标明删除条件。

## Comment Audit

触碰以下边界时，检查附近注释是否仍能让 maintainer 回答“谁拥有它、为何不在另一层、谁能做
副作用、grant 在哪里消费、fallback 何时删除”：

- `crates/tools/src/lib.rs`：为什么 concrete tools crate 不拥有 tool abstraction/registry。
- `crates/tools/src/file/mod.rs`：tool 只 parse、调用 access、shape output；fs side effect 属于
  `loong-access::fs`。
- `crates/loong-runtime/src/runtime/`：Runtime 是 Kernel、typed plane 与全部 live Session 的长期 owner，
  不是 UI state。ordinary handle 不暴露 raw Kernel；kernel/core bearer fallback 只能位于可机械 allowlist
  的 runtime legacy ingress module。app-only legacy tool 仍由 app ingress dispatch，runtime 不通过
  callback 反向调用 app；两类 owner 都写明删除条件。
- `crates/loong-runtime/src/tool_plane.rs`：path/slot 属于 concrete registry，首次 resolve 为什么绑定
  concrete entry，以及 registry storage 为什么不拥有 post-grant dispatch。普通 caller 使用
  `ctx.tool(...).invoke(...)`；只有 runtime invocation wrapper 能消费
  `Granted<ToolInvocationAction>` 并调用 bound entry。`ToolInvocationContext` 附近说明它为何是 generic
  invocation 的真实 requirement、authority 只能取交集，以及为什么 trait 不能构造 root Context。
- `crates/access/src/<domain>`：为什么 public members 在 owning Access facade re-export、为什么
  concrete operation 与 Action 共置，以及共享 access/path chain 与最终副作用的边界。
- `crates/kernel/src/access.rs`：`KernelAccess<C>` 为什么在 kernel；runtime concrete Context 的
  `access()` 必须经 Runtime 绑定私有 kernel，普通 caller 不直接调用 `AccessCx::new`。
- `crates/loong-runtime/src/context/` / `session/`：分别解释 recursive execution scope 与 owned live
  Session；root Context 只由 Session runner private 构造，child authority 只能取交集，普通 handle
  不能重建 root Context。`Context::tool` 保持薄入口，不复制 runtime invocation orchestration。
- `crates/loong-runtime/src/invocation/`：说明 public `InvocationImpl` 为何保留 concrete associated
  types、private `ErasedInvocation` 为何只服务 Session mailbox，以及 runner 如何在 app code 外强制
  lifecycle/cancel/audit/finalization。附近必须明确禁止 global program factory、`Any`/`Value` envelope
  和 `Runtime<P>` 泛型传播。
- `crates/app/src/conversation/` 的 concrete invocation owner：说明它只实现一次 app algorithm
  entry，为什么现有 provider/context-engine contracts 留在 app 内部，以及为什么不能把
  `ConversationRuntime` 整体注册到 runtime。
- `crates/app/src/tools/plane.rs`：app 只组装 builtin registry/policy，plane primitive 属于
  `loong-runtime`；registration failure 必须传播。
- `crates/app/src/tools/mod.rs`：typed ingress 只做一次 lookup/fallback decision，以及该临时 bridge
  的删除条件。`tool_dispatch.rs` 只服务已明确选择的 legacy path，不能重新进入 `ctx.tool(...)`。
- `crates/loong-core/src/policy/engine.rs`：`PolicyEngine::grant` 是唯一 typed grant owner，core
  algorithm 对外不可覆写；不要增加只转发它的 Kernel/helper method。mandatory authorization
  evidence write、identity allocation 及二者同时失败都在 mint 前通过 source-preserving typed error
  传播；compound error 不能只保留其中一个 cause。
- `crates/kernel/src/kernel.rs`：legacy pack/token authorization 与 typed policy grant 的边界；
  generic operational recorder 与 grant-bound execution recorder 都负责 clock/event id/sink write，附近
  短注释解释各自接受哪些 evidence，以及为何不是 forwarding helper。Kernel 不 dispatch typed tool，
  domain side effect 仍需自己的 action grant。
- `crates/contracts/src/audit_types.rs`：不把 ToolPlane registry key、route 或 fallback 语义固化
  成 kernel contract。
- `crates/loong-core/src/policy/action.rs`：payload 是 action type-erased view，不是 legacy request。
- `crates/loong-core/src/policy/grant.rs`：grant metadata/report 与不可伪造 execution port。
- streaming gateway/provider boundary：client disconnect 只取消当前 Invocation；finalization 与强制 abort
  的顺序必须写清楚。

## Tests

- tests 放在对应 module 下；大 `tests.rs` 按 domain 行为拆成 sibling test modules。
- typed path 测试断言 generic authorization evidence；runtime ToolInvocation 另断言与被消费的
  `Granted.id()` 关联的 execution evidence。generic Access execution evidence 属于独立后续目标，
  不能用当前 `Granted<Action>::run(ctx)` 测试伪称已经存在。legacy path 只断言 legacy audit。
  禁止 `PlaneInvoked | ToolInvocation` 这类宽松双断言。
- policy tests 覆盖 pre/action/fallback 顺序、四种 decision、default deny、typed match、完整 report
  和 capability gate before policy。
- sealed grant/audit contract 跨 core/contracts/kernel 时必须在同一原子提交迁移全部 implementor 和
  tests；attempt/grant identity allocation + failure-evidence write 的双故障必须分别测试两个 source。
  禁止 `no-op audit`、`unbound-success` 或不可编译的中间提交。
- direct Access 回归测试必须读取 evidence collector，断言 action 顺序、terminal outcome 与 grant id；
  只记录 action kind 而不检查 evidence 不足以证明 mandatory authorization audit。
- cancellation tests覆盖 downstream disconnect、provider stream drop、tool 不再启动、finalization、
  grace timeout 和 Session 可继续执行。
- invocation erasure tests 必须用至少两个不同 concrete `InvocationImpl` 证明同一 Session mailbox 可
  返回各自 typed event/output/error；另测 audit failure、receiver drop 与 cancellation 都由 private
  runner 收口，concrete implementation 无法绕过 wrapper。
- 每个提交至少运行受影响 crate 的定向测试、`cargo fmt --all -- --check` 和 `git diff --check`；
  提交前按仓库 gate 使用本机 `cargo` 跑 workspace clippy/default/all-feature tests，不使用自动安装
  toolchain 的 wrapper。
