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
- concrete tool 只约束获取 governed access/tool invocation 所需的小 trait；不依赖 app concrete
  Context，也不直接依赖 fs path-policy view。
- concrete tool 可以通过 `ctx.tool()` 编排 nested typed tool，但物理 side effect 只通过
  `ctx.access()` 进入 owning Access boundary。

## Grant 与 Payload

- `ActionGrant<A>` / `Granted<A>` 的 mint 保持 core-private；执行入口消费 grant。
- `Granted<A>::as_ref()` 只用于消费前 audit/metadata inspection，不能扩成复制或绕过 API。
- 当前 outer `ActionGrant<A>` 已有 id/info；当前 goal 不修改 `Granted<A>` 的字段形状，也不新增
  `grant_id()`。runtime ToolInvocation 保留 outer metadata，直到关联 execution audit 结束。
- `ActionMeta::payload(&self) -> Cow<'_, Value>` 没有默认值。已有 JSON 返回 borrowed view，
  按次构造的 structured view 返回 owned value。
- legacy `ToolCoreOutcome` conversion 只存在于仍拥有 legacy envelope 的 app boundary；不能放进
  concrete tool、ErasedTool 或 Access。

## Helper

保留 helper 前必须同时满足：

1. 至少统一多个真实重复调用面；
2. 该约束不适合由类型、trait bound、owned struct、Action、Context 或 Granted typestate 表达；
3. 附近注释说明为什么需要 helper、为什么属于当前模块、是否为迁移期边界。

只搬运同构数据、包装 `from`/`into`、拼 legacy display alias、隐藏 policy/access construction、
或把应删除 boundary 延长一层的 helper 直接删除。

`loong-runtime::ToolInvocationContext` 不是上述搬运 helper：它是 runtime-owned
`ToolInvocation` 与 app-defined Context 之间唯一的跨 crate requirement，只允许 typed child
authority narrowing。不要给它增加 kernel/audit/runtime accessor、base constructor 或其它
Context convenience method。

Kernel 现有 generic `record_audit_event` 也不是 forwarding helper：它集中拥有 clock、event id 分配
和 sink write，并将 error boundary 收敛为 typed `AuditError`，向调用方保留失败 source。runtime
只能调用这个 governance recorder，不能读取 kernel-private state，也不能增加 `AuditHandle`、
route/receipt 或 `ctx.audit`。

## Error

- 每个 Access domain 拥有自己的 typed error，并用 `thiserror` 保留 source chain。
- 不为单个 `?` 新增无意义 `From`；只有跨多个调用点稳定表达同一层 error boundary 时才实现
  conversion。
- policy denial 通过 typed authorization error/report 传播，不靠字符串分类 helper。
- composite `ToolInvocationError` 只能在 runtime `ToolInvocation` wrapper 真正接入 lookup、caps
  override、child narrowing、grant、dispatch 和 execution audit 的同一提交中定义并使用；不能先
  发布未接线 scaffold。它保留 `CapabilityNarrowingError`、`PolicyGrantError`、`AuditError` 与 plane
  error source，不能中途转换成 `KernelError` 或字符串再分类。
- legacy string envelope 的最后转换只发生在 legacy owner，并标明删除条件。

## Comment Audit

触碰以下边界时，检查附近注释是否仍能让 maintainer 回答“谁拥有它、为何不在另一层、谁能做
副作用、grant 在哪里消费、fallback 何时删除”：

- `crates/tools/src/lib.rs`：为什么 concrete tools crate 不拥有 tool abstraction/registry。
- `crates/tools/src/file/mod.rs`：tool 只 parse、调用 access、shape output；fs side effect 属于
  `loong-access::fs`。
- `crates/loong-runtime/src/runtime.rs`：Runtime 是 kernel + typed plane 的长期 owner，不是 kernel
  facade 或 UI state。
- `crates/loong-runtime/src/tool_plane.rs`：path/slot 属于 concrete plane，ErasedTool dispatch
  必须消费 `Granted<ToolInvocationAction>`，raw dispatch 只在 runtime 内可达；普通 caller 使用
  `ctx.tool(...).invoke(...)`。`ToolInvocationContext` 附近注释说明它跨 crate 表达 child
  authority narrowing，不是搬运 helper。
- `crates/kernel/src/access.rs`：`KernelAccess<C>` 为什么在 kernel，以及 `AccessCx::new` 只能由
  concrete Context 的 `access()` 集中调用。
- `crates/app/src/context.rs`：迁移前标明旧 COW 形状和删除目标；迁移后只使用
  `Context<'a>` / `RuntimeContextFactory`，并解释 recursive execution scope、authority narrowing
  和 cancellation inheritance。`Context::tool` 保持薄入口，app Context 只直接实现 runtime-owned
  `ToolInvocationContext`，不复制 runtime invocation orchestration。
- `crates/app/src/tools/plane.rs`：app 只组装 builtin registry/policy，plane primitive 属于
  `loong-runtime`；registration failure 必须传播。
- `crates/app/src/tools/mod.rs` / `tool_dispatch.rs`：typed ingress 与 legacy fallback 的边界和
  删除条件。
- `crates/loong-core/src/policy/engine.rs`：`PolicyEngine::grant` 是唯一 typed grant owner，core
  algorithm 对外不可覆写；不要增加只转发它的 Kernel/helper method。mandatory authorization
  evidence write、identity allocation 及二者同时失败都在 mint 前通过 source-preserving typed error
  传播；compound error 不能只保留其中一个 cause。
- `crates/kernel/src/kernel.rs`：legacy pack/token authorization 与 typed policy grant 的边界；
  `record_audit_event` 负责 clock/event id/sink write，附近短注释解释它为何不是 forwarding helper。
  Kernel 不 dispatch typed tool，domain side effect 仍需自己的 action grant。
- `crates/contracts/src/audit_types.rs`：不把 ToolPlane registry key、route 或 fallback 语义固化
  成 kernel contract。
- `crates/loong-core/src/policy/action.rs`：payload 是 action type-erased view，不是 legacy request。
- `crates/loong-core/src/policy/grant.rs`：grant metadata/report 与不可伪造 execution port。
- streaming gateway/provider boundary：client disconnect 只取消当前 Turn；finalization 与强制 abort
  的顺序必须写清楚。

## Tests

- tests 放在对应 module 下；大 `tests.rs` 按 domain 行为拆成 sibling test modules。
- typed path 测试断言 generic authorization evidence；runtime ToolInvocation 另断言与保留的 outer
  `ActionGrant.id` 关联的 execution evidence。generic Access execution evidence 属于独立后续目标，
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
- 每个提交至少运行受影响 crate 的定向测试、`cargo fmt --all -- --check` 和 `git diff --check`；
  提交前按仓库 gate 使用本机 `cargo` 跑 workspace clippy/default/all-feature tests，不使用自动安装
  toolchain 的 wrapper。
