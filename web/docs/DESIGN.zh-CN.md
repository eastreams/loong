# LoongClaw Web 设计进度

## 1. 当前定位

LoongClaw Web 现在是一个可实际使用的本地 Web Console，而不是单纯演示壳子。

当前它主要承担四件事：

- 作为本地 runtime 的 Web 入口
- 承接 onboarding 与最小配置写入
- 展示 runtime / tools / abilities 的真实状态
- 提供一个比 CLI 更轻的日常聊天与排障界面

当前阶段仍然是“开发态优先、产品态逐步收口”，不是完整开箱即用的远程 Web 产品。

## 2. 架构方向

### 当前开发态

当前开发仍以“分离式前端 + 本地 API”为主：

- 前端：Vite dev server
- 后端：本地 daemon Web API
- 认证：本地 token / pairing

这样做的原因很直接：

- 前端迭代更快
- 联调成本更低
- 热更新体验更稳定

### 当前产品态方向

产品态已经开始往“同源收口”推进：

- 本地同源静态模式已可用
- `web install / status / remove` 已可用
- same-origin 模式优先走本地 session cookie，而不是手动 token

一句话：

> 现在保留分离，是为了开发效率；长期方向仍是本地同源体验更顺。

## 3. Onboarding 进度

### O1：首次进入状态检测

已完成第一版。

当前已有：

- `GET /api/onboard/status`
- runtime / token / config / provider 状态聚合
- 首屏 onboarding 状态面板
- ready 状态放行

### O2：最小 provider 可写配置

已完成，并同时接入 onboarding 与 Dashboard。

当前可写项：

- provider kind
- model
- base URL / endpoint
- API key

### O3：验证与放行

已完成 apply-and-validate 主链路。

当前行为：

- `POST /api/onboard/validate` 负责最小验证
- `POST /api/onboard/provider/apply` 负责原子 apply
- Dashboard 不再因为 apply 把用户强制踢回 onboarding
- kind-route 明显错配时，后端会直接拒绝保存

### O4：token / pairing 收口

当前属于“已分流，但还没完全收口”：

- 开发态仍保留 token / pairing
- 同源产品态优先走 session cookie
- onboarding 已能处理自动配对、手动 token、session refresh

### O2.5：轻配置项补齐

已完成第一版。

当前已支持：

- personality
- memory profile
- prompt addendum

当前边界：

- 不开放完整 prompt / tools / memory 底层参数
- Dashboard 还不是完整配置控制台

## 4. 页面分工

### Chat

Chat 当前更聚焦“这轮对话最需要看到什么”。

已具备：

- 多会话
- 可见消息历史
- 流式输出
- 会话级临时视图态缓存
- 更友好的最近活跃时间
- 会话本地名覆写
- 路由级 keep-alive
- 发送失败时输入恢复
- 流式失败但 turn 已 accepted 时不误删用户消息

这一轮比较重要的变化是：

- Chat 已消费后端 `turn.phase`
- 前端会把 `turn.phase`、`tool.started`、`tool.finished` 与 `message.delta` 合并映射成更轻的 `streamPhase`
- 也就是说，Web 聊天状态已经不再只是一个模糊的“生成中”

### Dashboard

Dashboard 当前更聚焦“本地实例按什么状态在跑”。

当前已提供：

- provider 状态
- runtime 状态
- connectivity 诊断
- config snapshot
- provider 最小写入
- preferences 轻配置项写入
- tools posture 摘要
- approval queue
- 只读 Debug Console

其中最近比较关键的补齐是：

- `approvalMode / autonomyProfile / consentDefaultMode / sessionsAllowMutation`
- approval queue 的显式展示
- external skills 风险姿态摘要

### Abilities

Abilities 现在已经不是简单的信息页，而是第三个主要页面。

当前承接：

- `Personalization`
- `Channels`
- `Skills`
- `Mascot`

当前重点变化：

- `Skills` 已改为以后端 runtime/catalog 真值为主，不再主要依赖前端内置映射
- 已能区分 visible tools、hidden surfaces、browser companion、external skills
- 已补自治/审批/consent 等运行时姿态摘要
- `Mascot` 目前是实验性开关入口，默认关闭

## 5. Debug / Runtime Console

当前已落地 Dashboard 内嵌的只读 Debug Console。

它不是浏览器终端，也不是 CLI 镜像，而是：

- 只读
- 分块展示
- 面向观测与排障

当前主要能看到：

- runtime snapshot
- 最近几次操作块
- process output 摘要
- 本轮是否发生真实 tool call

## 6. 最近这轮值得记住的更新

- onboarding 与 Dashboard 共用 provider catalog
- provider apply 改成当前页验证，不再强制回 onboarding
- Dashboard 已补 approval queue
- Dashboard 已补 autonomy / approval / consent / session mutation 等工具姿态
- Abilities `Skills` 已切到 runtime truth
- Abilities 已增加 `Mascot` 实验入口
- Chat 已消费 `turn.phase`，流式状态更真实

## 7. 当前边界

当前仍未完成：

- 更完整的 cancel / reconnect / resume
- 完整 tool trace / event timeline
- 更完整的 Dashboard 受控写入
- 更接近真实 CLI 的连续输出流 Debug Console

另外有两点需要明确：

- 小宠物目前仍是前端实验能力，不是一个后端常驻 agent
- Web 侧还没有把 browser companion、approval、页面操作等能力真正接成“小宠物 agent 行为”

## 8. 当前优先级判断

如果继续沿当前方向推进，Web 更值得优先跟的仍然是：

1. 聊天流式 turn 生命周期和错误恢复
2. Dashboard / Abilities 对 runtime truth 的可解释展示
3. 小宠物的视觉、布局和聊天状态联动

而不是先把 Web 做成完整配置后台。
