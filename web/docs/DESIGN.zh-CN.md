# LoongClaw Web 设计进度

## 1. 当前定位

> 最后更新：2026-03-23（已补 CI 相关 review 修复）

LoongClaw Web 现在已经不是一个纯壳子，而是一个可实际使用的本地 Web Console：

- 可以进入 `chat / dashboard`
- 已接入真实 runtime
- 已有 onboarding 首屏检查与放行
- 已支持最小 provider 配置写入与验证
- 已支持轻配置项写入（personality / memory / prompt addendum）
- 已支持 token 轻自动配对与 same-origin session 模式
- 已有一个可用的 Dashboard Debug Console 原型
- 已实现 `web install / status / remove`

当前它已经具备：

- 开发态分离运行
- 同源静态产品态骨架
- 第一版可选安装

但它仍然处于 **开发态优先、产品态逐步收口** 的阶段，还不是完整的“开箱即用 Web 产品入口”。

## 2. 架构方向

### 当前开发态

当前仍然采用：

- 前端 dev server
- 本地 API
- 本地受保护 runtime

也就是“分离式前端 + 本地 API”的开发结构。这样做的原因是：

- 前端迭代快
- 联调成本低
- Vite 热更新体验好

### 长期产品方向

如果后续要做：

- 可选安装
- 官方 host
- 更顺滑的首次进入体验

那么 Web 更适合逐步收敛到 **同源设计**。

当前状态已经是：

- 开发态：继续允许分离，保持开发效率
- 本地产品态：已支持 `same_origin_static` 第一版
- 鉴权：开发态保留 token / pairing；同源产品态已切到更轻的本地 session cookie 心智
- 安装态：已具备第一版 `install / status / remove`
- 长期方向：继续减少 token / pairing 的显式负担，把产品心智收敛到同源 session

一句话：

> 现在保留分离，是为了开发快；当前已具备第一版同源入口；以后继续把同源体验做顺。

## 3. Onboarding 进度

### O1：首次进入状态检测

已完成。

当前已有：

- `GET /api/onboard/status`
- runtime / token / config / provider 状态聚合
- 首屏 onboarding 状态面板
- ready 状态确认进入

### O2：最小可写配置

已完成第一版，并已同时接入 onboarding 与 Dashboard。

当前 Web 可写的最小配置项为：

- provider kind
- model
- base_url / endpoint
- api key

这条写入链当前被：

- onboarding 首屏
- Dashboard `Provider Settings`

共同复用。

### O3：验证与放行

已完成第一版，并已补上当前页验证与失败回拉。

当前已有：

- `POST /api/onboard/validate`
- `POST /api/onboard/provider/apply`
- provider 配置“应用并验证”原子路径

当前验证关注的最小问题是：

- endpoint 是否可达
- 凭证是否通过基础探测

Dashboard 里现在不会再因为 `Apply` 把用户踢回 onboarding，而是：

- 留在当前页
- 弹出“正在验证 / 验证成功 / 验证失败”的短时反馈
- 验证失败时回到修改前状态
- 若 provider 已应用、preferences 失败，会回拉真实状态并给出更准确的错误语义

### O4：token / pairing 收口

当前属于 **部分完成，开发态与产品态已经分流**。

已完成：

- token 配对已收进 onboarding 面板
- 不再依赖单独的顶部 token banner
- Web 会优先尝试一次轻量自动配对
- 自动配对成功后，通过本地受信 cookie 建立当前浏览器的配对状态
- 自动配对失败时，会回退到手动输入 token
- 手动 token 输入框不会因为自动配对尝试而消失

当前边界：

- 安装态 / 同源产品态虽然已经不再把 token 当成主路径，但还不是彻底无感配对
- 分离开发态仍然会在必要时暴露 token 文件路径
- same-origin 模式下已新增 `session_refresh` 分支，用于本地 session 失效后的页面刷新恢复
- 开发态仍然需要处理本地 API token 的概念

### O2.5：轻配置项补齐

已完成第一版落地，并已接入 Dashboard 写入。

当前已支持：

- onboarding 首屏新增“可选个性化设置”折叠区
- 可在首次进入时按需设置：
  - `personality`
  - `memory_profile`
  - `prompt addendum`
- Dashboard 中也可编辑并保存同一组轻配置项

当前边界：

- `system_prompt` 仍不可直接修改
- 当前仍不开放更重的 prompt / tools / memory 底层参数
- Dashboard 虽已支持最小写入，但还不是完整的配置控制台

## 4. Dashboard / Chat 分工

### Chat

当前更聚焦“这轮对话时最该看到的信息”：

- 当前模型
- 记忆窗口
- 生成中状态
- 流式输出
- 会话列表 / 历史消息

补充：

- 输入交互已改为：`Enter` 发送，`Shift + Enter` 换行
- Chat 历史现在按**可见消息**计数，不让内部记录占掉消息泡额度
- 多 session 已可用；会话上下文彼此独立，但底层运行配置仍是全局共享
- Chat 现在已移除早期的 Web 侧临时 `toolAssist` workaround，工具发现与后续跟进主要依赖 runtime / provider 主链路本身
- `chat / dashboard` 已加路由级 keep-alive：切页返回后可保留进行中的可见状态
- 会话切换已补齐每个 session 的临时视图态缓存（最新问话、思考中状态、流式占位消息、tool 状态）
- 消息区滚动行为已修正为“锁在聊天框内滚动”，避免消息把整页撑长
- 流式失败时若 turn 已被后端接受，前端不再误删该轮用户消息
- 发送失败时会恢复输入框内容

### Dashboard

当前更聚焦“本地实例按什么配置在跑”：

- provider 状态
- runtime 状态
- connectivity 诊断
- 本地配置快照
- Provider Settings 最小写入
- preferences 轻配置项写入
- 工具运行态概览
- Debug Console 入口

## 5. Debug / Runtime Console

### 当前状态

已落地一个 **Dashboard 内嵌的只读 Debug Console 原型**。

它不是完整浏览器终端，也不是可交互 CLI，而是：

- 只读
- 终端风格
- 面向观测和排障

当前可以看到：

- runtime snapshot
- 最近几次操作块
  - 对话 turn
  - provider apply / validate
  - preferences apply
  - token pairing
- 简化过的 process output
- 本轮是否发生真实 tool call 的直接提示

当前设计重点已经从“把卡片塞进终端皮肤”调整为：

- 一次操作一段反馈
- 更像只读 CLI 输出块
- 内容在窗口内滚动，不拉长整个页面

### 还没做到的

- 真正的 CLI stdout 原样镜像
- 完整的连续事件流
- 多 session 并行调试视图
- 更细的 turn / provider / tool 历史筛选

## 6. Provider / Tool / Routing 诊断

最近这轮开发已经证明，很多问题不能简单当成“Web bug”。

### Provider transport

尤其是 Volcengine / Ark 这类 host，在代理 / TUN / fake-ip 环境下会出现：

- 短请求偶发成功
- 稍长 completion 更容易失败
- Web 和 CLI 都会继承同一条 provider transport 问题

因此当前已经补上：

- provider host DNS 解析检查
- fake-ip 命中判断
- endpoint 基础 probe
- route guidance

### Tools

当前还存在一个重要产品/运行时问题：

- `tool.search` 对中文和泛化工具意图的召回不足
- 用户即使明确说“请使用 shell / file 工具”，模型也常常并没有真的发起工具调用
- Debug Console 现在已经能明确显示：
  - 本轮有没有真实 tool call
  - 还是模型只是口头说“我检索过了”

这部分当前更像 runtime / tools 侧问题，而不是单纯 Web 问题。

## 7. 近期新增事项

这段时间新增且值得记录的事项：

- Dashboard `Provider Settings` 已接到真实写入接口，不再只是壳子
- provider apply 改成”当前页验证”，不再强制回 onboarding
- Dashboard 工具区已对齐上游新增能力：
  - `web_search`
  - `browser_companion` 运行态
  - `file_tools` 聚合项（覆盖 `file.edit`）
- Mac 端已补 `start-dev.sh / stop-dev.sh`
- 同源静态模式脚本已补齐：`start-same-origin.* / stop-same-origin.*`
- 顶部导航已支持语言切换与明暗主题切换
- Debug Console 已支持更像”按操作分段”的展示
- Chat 历史显示已修正为按**可见消息**计数
- `chat / dashboard` 已加入路由级 keep-alive，切页返回可保留进行中可见状态
- 会话切换已补齐临时视图态缓存，减少”最新问话/思考态丢失”
- Chat 消息区滚动链路已修复为容器内滚动，避免整页被消息撑长
- Chat 发送失败时会恢复输入框内容，不再直接吞掉用户 prompt
- 流式失败时若 turn 已被后端接受，前端不再误删该轮用户消息
- 新建会话首条消息失败时，空白会话会被及时清理
- Dashboard `Apply` 在 provider 成功、preferences 失败时会回拉真实状态，不再让 UI 和实际配置分叉
- onboarding 现在会区分 `runtime offline` 与 `401 / session_refresh / token invalid`
- **`web install/status/remove` 命令已实现**（`crates/daemon/src/web/`）：
  - `loongclaw web install --source <dist-dir>`：将构建产物复制到 `~/.loongclaw/web/dist/`，并写入 `install.json` 清单
  - `loongclaw web status`：输出安装状态、安装时间与来源路径
  - `loongclaw web remove [--force]`：清理安装目录
  - `loongclaw web serve` 现在会自动检测 `~/.loongclaw/web/dist/index.html`；检测到时无需传 `--static-root` 即可进入同源模式

## 8. 当前已知边界

当前仍未完成：

- 所有 provider 路径的统一真流式
- cancel / reconnect / resume
- 完整 tool trace 面板
- 更完整的 memory / tools / prompt Web 写入
- 更完整的 Dashboard 受控写入
- 安装态级别的自动 token 配对（`web install` 已落地，但同源无感配对尚未打通完整链路）
- 更像真实 CLI 的连续输出流 Debug Console
- `tool.search` 的中文 / 泛化意图召回问题

## 9. CI 相关 review / 修复结果（2026-03-23）

这轮围绕 CI 与 reviewer 反馈，优先收掉了会直接影响真实使用、状态一致性或安全边界的问题，而不是继续叠加新功能。当前已经修复并验证通过的项包括：

- Chat 流式失败时，若 turn 已被后端接受，前端不再误删该轮用户消息
- Chat 发送失败时会恢复输入框内容
- Dashboard 在 provider 成功、preferences 失败时会回拉真实状态，避免 UI 与实际配置分叉
- onboarding 不再把 `401 / session 失效 / token 无效` 误判成 `runtime_offline`
- `ConversationRuntime` 对 `event_sink` 的扩展改为兼容式 API，不再直接制造 public trait breaking change
- discovery-first follow-up 继续透传 ACP event sink，多轮工具发现链路不再丢 tracing
- Web API 生产路径改用 `bootstrap_kernel_context_with_config(...)`，不再默认走临时 in-memory audit
- `same_origin_static` 的写操作校验已从“任意 loopback origin”收紧为 daemon 自己的 exact origin
- Dashboard 表单已补 dirty tracking，轮询或刷新不会覆盖未保存编辑
- Dashboard 数据加载已补“只认最新请求”的保护，旧请求不会回写过期状态
- `config_invalid` 状态下重新开放 provider 修复入口，避免把用户卡死在 onboarding
- Dashboard 样式中已替换过时的 `word-break: break-word`

这轮 review 里也有几项我们判断为**值得记账，但不阻塞当前 Web 主线**的问题，后续会继续收口：

- onboarding 的 provider/preferences 写入仍然是 direct config write，尚未纳入统一 kernel capability / policy / audit 路径
- recent-session 枚举目前仍走 direct SQLite path，而不是 first-class memory op
- `api_only` 开发态 token 目前已从 `localStorage` 收敛到 `sessionStorage`，长期仍可继续往更短暂的内存式凭证收敛
- Debug Console 日志 tail 已改成按文件尾部字节读取；后续仍可继续优化连续流展示与日志分类

## 10. 当前仍值得优先关注的事项

当前最值得继续推进的是：

- `streamTurn` 的中断与提前关闭语义仍值得继续补强
- `ChatPage`、`DashboardPage`、`OnboardingStatusPanel` 仍偏大，后续继续优先抽 hook / 子组件
- Chat 页仍有少量硬编码英文字符串，需要继续补齐 i18n
- Debug Console 仍需从“可用原型”进一步推进到更像连续只读输出流
- 安装态与同源产品态第一版骨架已经具备，后续重点会转向更顺的安装体验与产品态文案/状态收口
