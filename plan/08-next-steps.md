# plan: 最小提交顺序

本文件只记录后续实现顺序、每步完成线和验证命令。长期原则见
`01-principles-and-boundaries.md`；截至 2026-07-11 的实现偏差见
`07-kernel-audit-and-deviations.md`。

每个编号项都是一个最小提交候选。除非某一步明确要求合并，否则不要把相邻步骤塞进同一个
commit。已完成的步骤从本文件删除，避免后续实现被过期完成线误导。

1. 继续迁移剩余 legacy side-effect tools：
   - `write` / `edit` 的 context/kernel-routed 调用已走 typed plane；无 context direct
     `write` / `edit` 已 fail closed；
   - `config.import` 的 kernel-routed/context-aware `plan` / `discover` / `plan_many` /
     `recommend_primary` / `merge_profiles` / `map_skills` 已通过 `ctx.access()` 读取
     import files、candidate directories、skills artifacts 和可选 output preview config；
     simple `apply` 已通过 `ctx.access()` 读取现有 output config 并写回最终 config；
     `rollback_last_apply` 已通过 `ctx.access()` 读取 manifest 并恢复/删除 output；
     `apply_selected` 在 `apply_skills_plan=false` 时已通过 `ctx.access()` 创建 state dir、
     写 backup、写 output config、原子写 import manifest；`apply_skills_plan=true`
     在 kernel-routed path 已 fail closed，不再 fallback 到 legacy direct side effect；
     只有 direct legacy API/测试路径里的 skills bridge 分支继续依赖 `FilePolicyExtension`
     的迁移期 guard；已迁移的 `config.import` modes 不再走 direct file preflight；
   - 不要只把 `config.import` 入口注册进 typed plane 来假装迁移：它调用的
     `migration::*` / `config::load` / `config::write` 当前会直接读写、备份、扫描文件。
     迁移完成线必须先把这些 I/O 抽到 access-backed port 或等价的 granted action run
     边界；
   - `config.import` 的迁移先拆 filesystem primitive，再迁 tool 入口。当前 `loong-access::fs`
     已有 read/write/atomic-write/copy-file/remove-file/remove-dir-all/rename/read-dir/glob/
     content-search/inspect-path/create-dir-all，仍不足以覆盖 skills lifecycle 的全部副作用：
     - discovery / plan：已有受治理的 file read、一层 directory scan、canonical/path
       metadata 基础；read-only modes 的 context-aware path 已接入；
     - simple apply：已用 access 读取现有 output config、渲染 config、写最终 output
       config；不再调用 `config::{load,write}`；
     - apply_selected：无 skills bridge 的路径已迁入 access；剩余 `apply_skills_plan`
       需要把 external skills manifest、managed install/remove、失败 rollback 迁到
       新的 access/tool 调用边界；
     - config codec：`config::parse` / `config::render` 已提供无 filesystem side effect 的
       解析/编码边界；`config::load` / `config::write` 仍是 legacy direct fs 调用点；
     - rollback：kernel-routed `rollback_last_apply` 已用 access 读取 manifest、复制
       backup、恢复 output，或删除不存在前 output；remove-file 只删除文件或 symlink，且
       final component 不跟随 symlink；
     - apply_selected failure rollback：无 skills bridge 的 config output restore 已有
       access-backed path；skills bridge rollback 仍要迁移。
   - 因此下一步 code 不是 `Register(ConfigImportTool)`，而是把 `apply_selected` 的
     `apply_skills_plan=true` 路径改成显式边界：external skills install/remove 不能继续
     作为 migration 内部 direct tool side effect；external skills manifest 写入也不能退回
     direct atomic write 或 `FilePolicyExtension`。`FilePolicyExtension` 现在只标记这个
     未迁移分支，不能再扩回已迁移 modes；
   - 当前 block 在 skills lifecycle，而不是 config import payload 解析：
     `skills.install` / `skills.remove` 仍是 legacy `ToolCoreOutcome` helper，内部有
     staging/copy/archive/extract/index/remove 等 direct filesystem side effects；rename 和
     remove-dir-all 已有 governed fs primitive，但还不能独自迁完 install/remove。先迁出
     skills lifecycle 的 typed/access 边界，再让 config import 的 skills bridge 通过
     `ctx.tool(...).invoke(...)` 或等价 access-backed boundary 调用；
   - `glob.search` / `content.search` 的 kernel-routed 调用已注册为 typed read-family
     path；无 context direct 调用已 fail closed，旧 app-local search helper 已删除；
   - 逐个工具迁移：concrete tool 只解析 payload、调用 `ctx.access()` / `ctx.tool()`、
     格式化 typed output；side effect 必须落在 access crate 的 granted action run 边界；
   - 迁移完成后删除 `FilePolicyExtension` 对应旧分支；
   - 逐步清空 `Kernel::execute_tool_core` 调用面，再删除 `LegacyToolPlane` 和 adapter
     trait；
   - 完成线：
     - migrated import 不直接调用 filesystem/network side effect；
     - side effect 只发生在 access crate 的 granted action run 边界；
     - kernel-routed `apply_selected + apply_skills_plan=true` fail closed，直到 skills
       lifecycle 迁完；
     - `FilePolicyExtension` 只覆盖 direct legacy `apply_selected + apply_skills_plan=true`，
       并在该分支迁完后删除；
   - 验证：按迁移工具分别跑对应 app/access 测试，再跑
     `cargo check -p loong-access -p loong-kernel -p loong-app -p loong` 和
     `git diff --check`。

2. 完成 runtime owner 与 session-owned unified context：
   - `loong_runtime::Runtime<AppContextFactory>` 已是 kernel 与 tool plane 的唯一 runtime owner，
     不再引入 app wrapper 或第二个 runtime 类型；原 `AppContextShared` 只是混装 runtime、pack、
     token 和 config 的无语义容器，已删除，`AppContext` 直接表达各字段的 ownership；
   - host surface 当前仍直接持有 root `AppContext`；下一步改为持有现有 `Arc<Runtime<_>>` 和
     app runtime config，在 session 创建时通过 kernel 已有的 pack registry/token issuer 构造
     自己的 `AppContext`，invocation 只从该 session context cheap-clone 派生 overlay；
   - conversation 另有一份 `SessionContext` 保存 session id、parent、tool view、workspace/skill
     roots、runtime narrowing 和 runtime-self continuity；这是当前剩余的第二个 source of truth；
   - `GovernedSessionMode::AdvisoryOnly` 是权限语义，不是“缺少 context”：advisory session
     也必须由同一个 runtime 构造 `AppContext`，但使用 kernel 签发的 scoped token，至少不能
     获得 `InvokeTool` 或 mutation capabilities；tool execution 由 capability/policy gate 自动
     fail closed，不能继续靠 `ConversationRuntimeBinding::AdvisoryOnly` 分支手动拦截；
   - 先把 host 的 bootstrap 拆成“构造现有 `Arc<Runtime<_>>`”和“为具体 session 签发 token、
     构造 `AppContext`”两个真实 ownership 阶段；随后删除 host/root `AppContext`、
     `Option<AppContext>`、`ConversationRuntimeBinding` 和 provider 的 no-context 对应物，不保留
     空 context、兼容 enum 或 advisory fallback；
   - 将 `SessionContext` 的 session/agent metadata、tool namespace view、roots、narrowing 和
     continuity 并入 `AppContext`，随后直接删除 `SessionContext`，不留 alias、wrapper 或
     `Option<session>` 兼容形状；
   - runtime owner 持有 kernel 和 tool plane，kernel 继续拥有 pack registry 和 token issuer；
     `AppContext` 持有 runtime 引用、当前 pack/token evidence、config 及 session/invocation 的
     不可变 view，child context 仍只能收窄 caps；
   - 完成线：turn/tool/access/policy/provider 从同一个 session-owned context source of truth 读取
     session、agent、tool view、caps、roots 和 continuity；host 不再把 root context 当作 session；
   - 验证：context/session/conversation/provider/tool execution 测试，workspace default/all-feature
     tests、strict clippy、architecture check、`git diff --check`。

3. 将 provider/runtime-self live-source 读取迁入 governed access：
   - `AGENTS.md` / `TOOLS.md` / `IDENTITY.md` 等 source discovery 已只产生 lexical 候选路径；
     文件存在性、canonical containment、symlink escape 和内容读取都由后续 fs access 决定；
   - provider source loader 已直接接收 `&AppContext`；`ProviderRuntimeBinding` 只在 prompt
     projection 入口决定 context-bound 或 advisory，不再向具体 loader 传播；workspace guidance
     和 runtime-self 共享唯一的 governed access read/text-normalization boundary；
   - 当前传入的仍是 host/root context。步骤 2 完成后必须改为 session 持有的 unified context，
     再删除 advisory/no-context prompt assembly 分支；不能把 host context 当作最终 session API；
   - context-bound 与普通 provider assembly 使用同一 context/access 路径，并同时产出
     `RuntimeSelfContinuity`；删除 no-kernel live-source fallback，而不是再增加 advisory bridge；
   - 删除 `TODO(deprecate-no-kernel-live-source)` 标记对应的旧分支；loader bridge TODO 已随
     loader 直接接收 context 删除；
   - 完成线：runtime-self 文件内容只在 granted fs action 中读取；provider 只消费读取结果并
     构造 prompt projection；typed path 有 generic action audit，但没有伪造 tool invocation audit；
   - 验证：provider request-message、context-engine、workspace-guidance、runtime-self continuity
     测试，以及 `cargo check -p loong-access -p loong-app -p loong`、`git diff --check`。

4. 收敛 ToolPlane ingress、catalog 与 display ownership：
   - 所有持有 unified context 的调用点直接使用 `ctx.tool(path)?.invoke(payload).await`；删除
     `execute_kernel_tool_request` 中只为 typed path 搬运 legacy envelope 的 bridge；
   - concrete tool descriptor 由 tool 实现提供，plane 在注册/list 时组合 path + descriptor；拆除
     app 全局 static catalog 中属于 concrete tool 的重复 metadata；
   - display name 由 plane-local path 的稳定 formatter 产生；删除 `file.read -> read` 这类 legacy
     display alias helper。真实 alias 若将来需要，必须作为 plane registration 语义单独设计；
   - 剩余工具完成步骤 1 的迁移后，删除 `ToolCoreRequest` / `ToolCoreOutcome` typed bridge、
     `Kernel::execute_tool_core` 调用面、`LegacyToolPlane` 和 adapter traits；
   - 完成线：新增 builtin tool 只需要 concrete `ToolImpl` 和一次 registration；typed dispatch、
     prompt catalog、search metadata 不再要求修改手写 match/helper；
   - 验证：tool plane/registry/catalog/search tests、每个 migrated tool 的调用测试、workspace
     default/all-feature tests、strict clippy、`git diff --check`。

5. 把 tool execution audit 从 contracts/kernel 收回 runtime owner：
   - `Kernel::grant` 继续强制记录 generic action authorization audit，包括 caps、policy report、
     allow/deny 和 grant id；这部分不能下沉到 ToolPlane 或 concrete tool；
   - grant 消费后的 tool completed/failed/input-error 由 `ToolInvocation::invoke` 在 app/runtime
     ownership 下强制记录。concrete `ToolImpl` 不获得裸 audit API；
   - 从 contracts/kernel 删除 tool-specific `AuditEventKind::ToolInvocation` 和
     `record_tool_invocation` schema；sink 只接收 generic envelope，app runtime payload 只保存
     stable path display、grant id 和 execution outcome；
   - legacy `PlaneInvoked` 随步骤 3 的 legacy plane 一起删除，不保留 typed/legacy 宽松双断言；
   - 完成线：deny 只由 generic grant audit 表达；grant 后 outcome 只记录一次；tool registry key
     和 route/fallback 语义不进入 contracts/kernel；
   - 验证：kernel grant audit、app typed tool completed/failed/input-error、daemon audit rendering
     测试，以及 workspace default/all-feature tests、strict clippy、`git diff --check`。

6. 收敛 Kernel/Policy contract：
   - 从 concrete `Kernel<C>` 提取调用方真正需要的稳定 governance trait；trait 只暴露 grant、
     token/pack boundary 和 audit authority，不暴露 typed tool registry 或 app runtime state；
   - legacy caller 清空后直接删除旧 `authorize_operation`、legacy kernel auth、legacy policy error
     和对应 helper；届时移除 `TODO(deprecate-legacy-kernel-auth)`、
     `TODO(deprecate-legacy-policy-error)`，不要只加 alias；
   - 完成线：新调用方只依赖稳定 governance trait 或 `PolicyEngine`；仓库没有旧
     authorization API 的 production caller；
   - 验证：kernel policy/grant/audit tests、all policy report ordering tests、workspace
     default/all-feature tests、strict clippy、architecture check、`git diff --check`。

7. 用 descriptor-relative fs backend 关闭 pathname TOCTOU：
   - 先记录跨平台 backend 决策并评估成熟库；优先采用经过验证的 descriptor-relative/
     capability-based filesystem primitive，不手写一套未经审计的 syscall wrapper；
   - path resolution 与 allowed-roots policy 仍是独立 action，但 `GrantedFsPath` 必须携带或引用
     后端可消费的稳定目录/对象能力，最终 read/write/remove/rename 不再按已授权 `PathBuf`
     重新解析 pathname；
   - target-following 与 final-component no-follow 语义必须继续由类型区分；Windows/Unix 无法提供
     完全相同保证时，差异要在 backend contract 和测试中显式表达，不能静默退回旧路径操作；
   - 完成线：authorization 与最终 side effect 使用同一受约束 handle/descriptor chain；并发替换
     symlink 或 ancestor 不能把操作重定向到 allowed roots 外；
   - 验证：access/kernel path policy tests、并发 symlink/rename race regression tests、各支持平台
     的 compile/test gate、workspace default/all-feature tests、strict clippy、`git diff --check`。

8. 按 owner 收敛 transitional crates 并更新架构文档：
    - 在 unified runtime 稳定后逐个审计 `loong-cli`、`loong-app-protocol`、`loong-runtime`、
      `loong-plugin-sdk`、`protocol` 和 `bridge-runtime`；每次只合并/删除一个 ownership 明确的
      forwarding shell，不按 crate 行数批量合并；
    - 先修正 kernel -> plugin-sdk 的反向依赖：kernel 所需 contracts 下沉到真正的 leaf，SDK
      只保留 plugin author-facing API；再处理 CLI/protocol/runtime transitional spine；
    - 每个 crate 变更同步更新 workspace dependencies、feature matrix、architecture checks、
      `AGENTS.md` / `CLAUDE.md` 镜像、crate DAG 和 public/release docs；
    - 完成线：没有只转发类型/函数的 phase spine 或 compatibility facade；workspace DAG 与文档
      一致且无 dependency cycle；`loong-runtime` 要么成为真实 runtime owner，要么该名字被删除；
    - 验证：受影响 crate 的定向测试、`cargo metadata` DAG 检查、architecture check、workspace
      default/all-feature tests、strict clippy、`git diff --check`。

## Code TODO 对照

代码里的迁移 TODO 必须能映射到上面的执行步骤；对应步骤完成时删除 TODO 和旧分支：

- `TODO(config-import-access)` / `TODO(access-migration)` -> 步骤 1；
- `TODO(session-owned-context)` -> 步骤 2；
- `TODO(deprecate-no-kernel-live-source)` -> 步骤 3；
- `TODO(tool-plane)` / `TODO(tool-plane-display)` / `TODO(tool-catalog-owner)` -> 步骤 4；
- `TODO(kernel-contract)` / `TODO(deprecate-legacy-kernel-auth)` /
  `TODO(deprecate-legacy-policy-error)` -> 步骤 6。
