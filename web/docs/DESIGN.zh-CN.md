# LoongClaw Web 设计说明

状态：Phase 4 进行中  
最后更新：2026-03-18

## 1. 产品目标

LoongClaw Web 不是新的 assistant runtime，而是现有 LoongClaw runtime 的一个本地优先前端表面。

首批目标：

- Web Chat
- Web Dashboard

它们都应复用现有 LoongClaw 的：

- conversation 语义
- provider 调用链
- tools
- memory
- audit / policy

## 2. 产品定位

Web 当前定位为：

- 可选前端模块
- 本地优先控制台
- 不替代 CLI

这意味着：

- 基础安装仍以 CLI/runtime 为主
- Web 是控制面与交互面扩展
- Web 与 CLI 在 provider/runtime 问题上共享同一条底层链路

## 3. 当前阶段判断

按当前真实实现看，项目已经不是“文档 + 壳子”阶段，而是进入了可用 MVP。

当前大致进度：

- Phase 1：完成
- Phase 2：完成语义复用，但未完全做 app 层抽象收口
- Phase 3：本地 API 控制面大部分完成
- Phase 4：前端 MVP 已跑通，继续打磨中

## 4. 当前已落地能力

### Web Chat

已具备：

- session 列表与 history
- 创建 session
- 创建 turn
- 流式 turn 消费
- assistant 下方生成中状态
- tool 事件摘要展示
- 简单 Markdown 标题 / 段落 / 列表渲染

### Web Dashboard

已具备：

- runtime 摘要
- providers 摘要
- tools 摘要
- config 摘要
- provider route / connectivity 诊断

### 鉴权与连接

已具备：

- 本地 token 鉴权
- token 缺失 / 失效状态提示
- Web API 元信息与连接状态展示

## 5. 当前 Chat 设计取向

Chat 设计方向已经从“静态聊天页面”转向“更像 agent 的工作面板”。

当前取向：

- 中间消息区尽量弱化卡片感
- assistant 生成状态直接挂在 assistant 消息下方
- turn 完成或失败后状态自动消失
- 工具事件先做轻量摘要，不做重型 trace 面板

当前的流式体验分两层理解：

1. 前端协议上已经是流式 turn
2. provider 层对 OpenAI-compatible chat completions 已优先尝试真实流式；不支持时退回缓冲路径

这意味着当前已经不再是纯“完整文本闪出”的伪流式，但也还没到完整统一的 provider 级流式能力。

## 6. 当前 Dashboard 设计取向

Dashboard 当前不是 BI 大盘，而是本地 runtime 控制台。

当前职责：

- 解释现在接的是哪条 provider/config/runtime 链路
- 提示工具策略和权限面
- 在 provider transport 失败时给出更可操作的路由诊断

设计上更偏：

- 摘要卡
- 配置快照
- 本地网络诊断
- 工具面板

而不是：

- 图表化分析
- 复杂运营数据看板

## 7. Provider 路由与 transport 诊断

最近这轮开发暴露出一个很重要的问题：

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

- provider 全量统一真实流式
- turn cancel / reconnect / resume
- 完整 tool trace 面板
- dashboard 受控写入
- runtime 抽象完全收口到 `crates/app`

这些仍属于下一阶段工作。

## 9. 下一阶段建议

### 4A：继续打磨 Chat 的 agent 表达

优先继续完善：

- 更完整的流式文本体验
- provider/retry 状态提示
- 更细的失败态表达

### 4B：继续补 Dashboard 的只读控制面

继续完善：

- provider health
- diagnostics
- route guidance
- runtime 细粒度状态

### 4C：谨慎推进 runtime 抽象收口

仍然需要，但应克制推进，优先避免和主线高频变动区域大面积冲突。

## 10. 设计结论

当前 LoongClaw Web 已经具备继续往“真正可用的 agent 控制台”演进的基础。

下一阶段重点不再是“有没有页面”，而是：

- 让 Chat 更像真实 agent
- 让 Dashboard 更像本地 runtime 控制台
- 让 provider / route / transport 问题更容易被理解和修复
