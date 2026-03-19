# LoongClaw Web 设计说明

状态：Phase 4 进行中  
最后更新：2026-03-19

## 1. 产品目标

LoongClaw Web 不是新的 assistant runtime，而是现有 LoongClaw runtime 的本地优先前端表面。

当前目标聚焦在两块：
- Web Chat
- Web Dashboard

它们都应复用现有 LoongClaw 的：
- conversation 语义
- provider 调用链
- tools
- memory
- audit / policy

## 2. 当前产品定位

Web 当前更接近：
- 本地优先控制台
- 已有 runtime 的操作与观察界面
- CLI 之外的可选交互表面

这意味着：
- 基础安装仍以 CLI / runtime 为主
- Web 现在还不是完整的首次进入入口
- Web 与 CLI 在 provider / runtime 问题上共享同一条底层链路

## 3. 当前阶段判断

按真实实现看，项目已经不再是“文档 + 壳子”阶段，而是进入了可用 MVP。

当前大致进度：
- Phase 1：完成
- Phase 2：已完成语义复用，但抽象收口仍应谨慎推进
- Phase 3：本地 API 控制面大部分落地
- Phase 4：前端 MVP 已跑通，并持续打磨中

## 4. 当前已落地能力

### Web Chat

当前已具备：
- session 列表与 history
- 创建 session
- 创建 turn
- 流式 turn 消费
- assistant 下方生成中状态
- tool 事件摘要展示
- 简单 Markdown 标题 / 段落 / 列表渲染

### Web Dashboard

当前已具备：
- runtime 摘要
- providers 摘要
- tools 摘要
- config 摘要
- provider route / connectivity 诊断

### 鉴权与连接

当前已具备：
- 本地 token 鉴权
- token 缺失 / 失效状态提示
- Web API 元信息与连接状态展示

## 5. Chat 设计取向

Chat 的方向已经从“静态聊天页”转向“更像 agent 的工作面板”。

当前取向：
- 中间消息区尽量弱化卡片感
- assistant 生成状态挂在 assistant 消息下方
- turn 完成或失败后状态自动消失
- 工具事件先做轻量摘要，不做重型 trace 面板

### 当前流式边界

当前流式能力分两层理解：
1. 前端消费链已经是流式 turn
2. 对 OpenAI-compatible chat completions，后端会优先尝试真实 provider streaming；不支持时再退回缓冲路径

因此当前已经不再是纯“完整文本闪出”的伪流式，但也还没有做到所有 provider 路径的统一真流式。

## 6. Dashboard 设计取向

Dashboard 当前不是 BI 大盘，而是本地 runtime 控制台。

当前职责：
- 解释当前接的是哪条 provider / config / runtime 链路
- 提示工具策略与权限面
- 在 provider transport 失败时给出更可操作的路由诊断

设计上更偏：
- 摘要卡片
- 配置快照
- 本地网络诊断
- 工具面板

而不是：
- 图表化分析
- 复杂运营数据看板

## 7. Provider 路由与 transport 诊断

这轮开发暴露了一个很重要的问题：

- 某些 provider，尤其是 Volcengine / Ark 这类 host，在 TUN / fake-ip / 代理环境下，短请求可能偶发成功，但稍长 completion 更容易失败

这不是单纯的 Web bug，也不是单纯的 provider bug，而是：
- provider transport
- 本地代理 / fake-ip / TUN
- timeout / retry 设置

共同作用后的结果。

因此当前设计上新增了一层本地诊断能力：
- 检查 provider host DNS 结果
- 检查是否命中 fake-ip
- 对 provider endpoint 做轻量 probe
- 给出 route guidance

这层能力的目标不是替用户自动改网络，而是：
- 更快定位问题
- 更少把 transport 问题误判成 Web 问题
- 为后续 `doctor` / `onboard` 提供可复用基础

## 8. 当前已知边界

当前仍未完成：
- provider 全量统一真流式
- turn cancel / reconnect / resume
- 完整 tool trace 面板
- dashboard 受控写入
- runtime 抽象完全收口到 `crates/app`

这些仍属于下一阶段工作。

## 9. 下一阶段建议

### 4A：继续打磨 Chat 的 agent 表达

优先继续完善：
- 更完整的流式文本体验
- provider / retry 状态提示
- 更细的失败态表达

### 4B：继续补 Dashboard 的只读控制面

继续完善：
- provider health
- diagnostics
- route guidance
- runtime 细粒度状态

### 4C：谨慎推进 runtime 抽象收口

这件事仍然需要，但应克制推进，优先避免和主线高频变动区域大面积冲突。

## 10. Web Onboarding 目标

当前 Web 更像“本地实例已经准备好之后使用的控制台”，还不是“普通用户首次进入产品的主入口”。

如果用户只是下载项目、安装并运行前端：
- 页面可以打开
- 但仍然缺少完整首次配置闭环
- 目前还需要本地 token 配对
- provider / model / key / endpoint 等关键配置也还不能在 Web 内完成

目标状态应该是：
- 用户启动 Web 后，先进入首次引导，而不是直接落到 chat / dashboard 壳子
- Web 能检查本地 runtime、token、config、provider readiness
- 用户能在 Web 中完成最基本的 provider 配置
- 完成保存与验证后，直接进入可用的 chat

换句话说，Web 后续要从“本地控制台”继续演进到“本地产品入口”。

## 11. 建议的 Onboarding 分阶段计划

### O1：首次进入状态检测（已落地）

先补一条专门的 onboarding 状态链路，用于回答：
- 本地 runtime 是否在线
- 本地 token 是否已完成配对
- config 是否存在
- provider 是否已配置到可用状态
- 当前缺的是哪一环

这一阶段的目标不是立刻可写，而是先把“为什么现在还不能用”清楚表达出来。

当前已经落地：
- `GET /api/onboard/status`
- 首次进入状态面板
- runtime / token / config / provider readiness 聚合展示
- ready 状态下的确认进入步骤

### O2：最小可写配置（第一版已落地）

在 Web 中补最小受控写入能力，第一版先覆盖：
- provider kind
- model
- base_url / endpoint
- api key

保存后需要能做一次基本验证，例如：
- provider endpoint 是否可达
- key 是否至少能通过最小探测

当前第一版已经具备：
- Web 端最小 provider 配置表单
- 受控写入 `POST /api/onboard/provider`
- 写入后自动刷新 onboarding 状态
- Dashboard 中的 Provider Settings 也已接入同一条最小写入接口
- 显式验证 `POST /api/onboard/validate`
- 保存 provider 配置后，先进入待验证状态，再决定是否放行进入 Web

当前第一版的边界：
- 只覆盖最小 provider 配置闭环
- 还没有把 CLI onboard 的轻配置项搬到 Web
- API key 这一步当前仍按“最小可用”优先，后续仍建议继续对齐到更稳妥的 env / secret 路线

这一阶段完成后，用户已经不必再手改 `config.toml` 才能先把 provider 跑起来。

### O2.5：补齐 CLI 的轻配置项

在 O2 主干稳定后，再补更偏“使用偏好”的轻配置项：
- personality
- memory_profile
- system_prompt / addendum

这一层适合继续沿用 Web onboarding，但不建议与 O2 第一版一起一次性塞入首屏流程。

### O3：验证与放行（已启动）

当前已经补上：
- `POST /api/onboard/validate`
- 保存 provider 配置后进入待验证状态
- 验证通过后才放行进入 Web

当前这一步的目标是把下面两件事分开：
- “配置已经写进去了”
- “这套配置已经足够健康，可以放用户进 chat”

当前验证策略仍然保持最小化：
- OpenAI-compatible chat providers 优先做一次最小聊天请求验证
- 其他协议族先走较轻的头部探测
- 验证结果回到 onboarding 面板，不再等到进入 chat 之后才暴露

### O4：进入 Chat 的最终放行条件

Web 只有在以下条件满足后，才应把用户放进主 chat 流程：
- runtime 在线
- token 已完成配对
- provider 配置已通过基础验证

当前已经开始把 token 配对收进 onboarding 面板，而不是继续依赖顶部独立提示条。

如果这些条件不满足，应继续停留在 onboarding，而不是把用户丢进一个“页面能打开但实际不可用”的状态。

## 12. 设计结论

当前 LoongClaw Web 已经具备继续往“可用的 agent 控制台”演进的基础。

下一阶段重点不再是“有没有页面”，而是：
- 让 Chat 更像真实 agent
- 让 Dashboard 更像本地 runtime 控制台
- 让首次进入流程真正闭环
- 让 provider / route / transport 问题更容易被理解和修复
