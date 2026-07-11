# plan: 跨模块变更约束

本文件列出跨模块 hygiene 规则。它不是任务列表；每个最小提交都要按这些规则检查。

## 后续变更约束

本节不是截至 2026-07-11 的状态清单，而是后续每个最小提交都要继续遵守的约束。

1. Cargo workspace dependency hygiene：
   - 新增或迁移内部 Loong crate 依赖时，先在根 `Cargo.toml` 的
     `[workspace.dependencies]` 加一条统一声明；
   - 叶子 crate 使用 `loong-*.workspace = true`，不要重复写
     `package/version/path`；
   - 历史 daemon/spec/bridge 依赖不要混进功能提交里大扫除，除非该提交正好触碰这些
     crate。

2. Concrete builtin tools crate hygiene：
   - `crates/tools` 只放具体 builtin tools；
   - 不在 `crates/tools` 定义或 re-export `ToolPlane`、`ToolImpl`、`RegisteredTool`、
     `Policy`、`AccessCx` 这类抽象；
   - app feature 只负责开关 concrete tool feature，例如
     `tool-file = ["loong-tools/file"]`；
   - feature flag 控制模块/工具族是否存在，不在模块内部保留 disabled stub；
   - 如果 `file` feature 关闭，`loong_tools::file` 就不应暴露；
   - 模块内部默认 feature 已启用，disabled/unavailable 错误由上层注册或路由处理；
   - concrete tool 需要 access 时，只约束 kernel 暴露的 context requirement，例如
     `for<'a> C::Cx<'a>: KernelAccess<C>`；fs-specific root views 由 access/fs 定义并经
     kernel 导出，不让 concrete tool 直接约束 path policy view。

3. Context access requirement hygiene：
   - `KernelAccess<C>` 这类 trait 是 concrete tools 获取 `ctx.access()` 的窄边界；
   - 它属于 kernel 公共边界，因为返回的是 kernel-defined `AccessCx`；
   - 它不放 `loong-access`，避免 app/spec/test 为实现 context requirement 反向依赖
     access；
   - 它也不放 `loong-core`，因为 core 不该知道 kernel access facade。

4. Grant inspection hygiene：
   - `Granted<A>` 可以提供只读 `as_ref()`，用于 audit 在消费 grant 前读取 action
     metadata；
   - 不能提供从外部构造或复制 grant 的 API；
   - 执行入口仍然消费 `Granted<A>`，例如 `Granted<A>::run(ctx)` 或
     `ToolPlane::invoke(Granted<ToolInvocationAction>, &ctx)`。

5. `ActionMeta::payload` borrowing hygiene：
   - 保持 `ActionMeta::payload(&self) -> Cow<'_, Value>`；
   - 不提供默认 `Null`，每个 action 都必须显式声明自己的 type-erased payload；
   - 对天然可借用的 action，返回 `Cow::Borrowed(&self.payload)`；
   - 对需要按次构造 JSON view 的 action，返回 `Cow::Owned(json!(...))`；
   - 后续 action 迁移不应复制旧的 owned `Value` 签名。

6. Helper function hygiene：
   - 默认先问“该约束能不能用类型表达”：例如 concrete action、context requirement trait、
     `Granted<A>`、`GrantedPath`、plane-local path/action、`ToolRegistration`；
   - 不为 legacy envelope、display alias、policy preflight、access construction、payload
     claim 之类边界残留新增 helper。先把转换留在 owning boundary；如果该 boundary
     本身应该消失，就在计划里迁移/删除，而不是扩写 helper；
   - 可以接受的 helper 必须满足两个条件：统一多处真实重复的调用形态；该调用形态不适合
     用类型、trait 或 owned struct 表达；
   - 每个保留的 helper 附近都要有短注释，说明它为什么不是类型、为什么放在所在模块、
     它是否是迁移期边界。没有这些理由，就内联或类型化替代；
   - code review 时发现 helper 只是在搬运同构数据、包装 `from`/`into`、隐藏 policy/access
     边界或制造 alias，应当直接删除并把逻辑放回 owning layer。

7. Comment audit hygiene：
   - 架构边界变更必须补少量注释，说明 ownership 和 why。注释不是“解释代码在做什么”，
     而是把只有迁移作者知道的设计约束写给后续 maintainer；
   - 每次碰下面文件时都要检查注释是否仍然准确：
     - `crates/tools/src/lib.rs`：说明 `loong-tools` 只放 concrete builtin tool
       implementations，不放 `ToolImpl` / registry / policy / access 抽象。注释必须讲清
       “为什么有一个 tools crate 却不承载 tool 抽象”，避免后来者把 plane 或 trait 又搬进来。
     - `crates/tools/src/file.rs`：说明 `ReadTool` 只负责 payload parse、调用
       `ctx.access().fs()` 下的具体 fs operation、格式化 response；文件读取、目录遍历、
       内容搜索副作用发生在 `loong_access::fs`，不能在 tool helper 里直接
       `std::fs::read` / `std::fs::read_dir`。
     - `crates/kernel/src/access.rs`：`KernelAccess<C>` 的注释必须讲清它为什么在 kernel
       而不是 core/access：它返回 kernel-defined `AccessCx`，并给 concrete tools 一个
       不依赖 `AppExecutionContext` 的窄 context requirement。
     - `crates/app/src/context.rs`：`AppExecutionContext::access()` / `KernelAccess` impl
       附近要讲清 `AccessCx::new(...)` 只应出现在 concrete context 的 `access()` 实现里；
       普通 tool/action 调用点应使用 `ctx.access()`，不要恢复 `kernel.access(ctx)`。
     - `crates/app/src/tools/plane.rs`：注释必须讲清 `loong-app::tools::plane`
       owns typed plane，kernel 不持有 typed registry；plane 按 path resolve，不按 payload claim；
       `invoke` 消费 `Granted<ToolInvocationAction>`，所以它是 granted primitive；普通调用
       点应使用 `ctx.tool(path)?.invoke(payload).await`，不要绕过 grant shortcut 直接执行。
     - `crates/app/src/tools/mod.rs`：typed dispatch 边界附近要讲清它只是迁移期
       orchestration：`ctx.tool(path)?` 做 lookup -> `ToolInvocation::invoke(payload)` build
       action -> kernel grant -> plane `invoke`；`read` 已由 aggregate `ReadTool` 接管，
       不要恢复按 payload claim/fallback 的 dispatch 设计。
     - `crates/kernel/src/kernel.rs`：generic `grant` 注释必须讲清 kernel 是 governance
       authority，不执行 typed tool；grant 过程负责 action authorization audit；tool
       invocation grant 只授权进入 `ToolImpl`，tool 内部 side effect 仍需自己的 access
       action grant。
     - `crates/contracts/src/audit_types.rs`：逐步只保留 kernel/sink 需要的 generic audit
       primitives。tool-specific execution audit 是 app runtime schema，不应在
       contracts/kernel 固化 `ToolInvocationRoute`、`ToolInvocationOutcome` 或 concrete
       ToolPlane registry key 类型。
     - `crates/loong-core/src/policy/action.rs`：`ActionMeta::payload()` 注释必须讲清 payload
       是 Action 的 type-erased structured view，不是 legacy bridge；签名是
       `Cow<'_, Value>`，并且没有默认 `Null`。
     - `crates/loong-core/src/policy/grant.rs`：`Granted<A>::as_ref()` 注释必须讲清它只用于
       grant 被消费前的 audit/metadata inspection，不能成为伪造、复制或绕过执行边界的入口。
     - `crates/app/src/tools/routing.rs`：context-aware direct read 注释必须讲清它只做
       direct-read payload normalization，然后进入 `ctx.tool("read")?.invoke(...)`；无 context
       legacy read 入口是统一 ctx/废弃旧入口之前的偏差，不能被提升成长期 routing 机制。
   - 注释验收标准：读者只看相关类型/函数附近的注释，就能回答“该层拥有谁”“为什么不在
     另一个 crate”“该 fallback 是否长期存在”“谁可以做副作用”“grant 何时被消费”；
   - typed path 测试断言 generic action grant audit + grant 后 tool execution audit；
     legacy path 测试只断言 legacy audit；
   - 不新增 `PlaneInvoked | ToolInvocation` 这种宽松断言；
   - 模块测试继续放对应模块下，例如 `tools/plane/tests.rs`、`file/tests.rs`。
