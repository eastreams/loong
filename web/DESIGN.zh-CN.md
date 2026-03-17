# LoongClaw Web Console 设计文档

状态：提议中  
范围：`alpha-test` 线，大型单 PR 交付  
最后更新：2026-03-17

## 1. 目标

为 LoongClaw 构建一个 Web Console，包含两个主要界面：

- Web Chat
- Web Dashboard

Web Console 必须复用现有 LoongClaw 的会话、provider、tool、memory 和 audit
语义。它是一个新的客户端界面，不是一个新的 assistant runtime。

## 2. 产品定位

Web Console 是一个可选前端模块。

- 基础安装仍然以 CLI 为主。
- Web 资源不进入默认安装路径。
- `onboard` 可以提供一个可选的 Web Console 安装项，并显示体积说明。
- 第一优先级是本地部署。
- 托管前端模式是后续分发选项，不是 MVP 要求。

## 3. 核心决策

采用 `local-first` 的前后端分离方案。

- 本地 `loongclaw` 进程仍然是系统真实执行入口和事实来源。
- Web Console 作为 `web/` 下的独立前端模块存在。
- 后端暴露本地 HTTP API 控制面。
- 前端消费该 API。

这样既能让 Web 具备独立演进能力，又能保证 CLI、Web 和未来 channel
都走同一条 runtime 路径。

## 4. MVP 非目标

- 不做单独的云端 agent runtime
- 不做多用户服务器模式
- 不默认暴露到公网
- 不承诺远程同步产品能力
- 不强制提供托管前端路径
- 不要求基础安装时自动安装 Web 资源

## 5. 仓库布局

建议结构如下：

```text
crates/
  app/
  daemon/
web/
  src/
  public/
  package.json
  DESIGN.md
  DESIGN.zh-CN.md
scripts/
  web/
docs/
  references/
```

理由：

- 在协议快速演进阶段，前后端放在同仓协作成本更低
- 避免过早拆仓带来的联调和发布协调成本
- 未来如果前后端发布节奏明显分离，仍然可以再拆分

## 6. 运行时架构

### 6.1 后端

后端职责放在 `crates/daemon` 和 `crates/app`。

`crates/daemon`：

- 启动本地 HTTP 服务
- 暴露 Web API 路由
- 执行 HTTP 鉴权和 origin 检查
- 可选地挂载本地已安装的静态资源
- 报告安装和运行状态

`crates/app`：

- 继续承载真正的 conversation runtime
- 提供一个可被 Web 和 CLI 复用的 session/turn service
- 持续负责 provider、tool、memory、ACP 和 audit 行为

### 6.2 前端

`web/` 下的前端是独立客户端。

职责：

- 渲染 chat UI
- 渲染 dashboard UI
- 管理本地连接状态
- 调用后端 API
- 处理本地 token 输入和存储

前端不能重写 runtime 逻辑。

## 7. 会话模型

Web 必须映射到现有的 conversation address 模型。

建议映射方式：

- `channel_id = webchat`
- `conversation_id = <browser_session_id>`
- `thread_id = <tab_or_subthread_id>`，后续需要时再加入

ingress 元数据可以包含浏览器上下文，但只能作为路由和 UX 辅助信息，
不能承担授权语义。

## 8. 主要界面

### 8.1 Web Chat

MVP 能力：

- 创建或恢复会话
- 发起一轮对话
- 读取最近历史
- 展示回复状态
- 展示当前 provider/runtime 简要状态

后续能力：

- 更高级的 trace 面板
- 协作式会话视图
- 更复杂的附件工作流

### 8.2 Web Dashboard

MVP 能力：

- runtime 摘要
- active provider 和 provider 可用性
- memory 状态
- tool 可用性和状态摘要
- 配置摘要
- doctor/runtime 警告
- Web Console 安装模式和资源状态

后续能力：

- 完整管理控制面
- 多实例视图
- 远程设备编排

## 9. API 形态

初始 API 面：

- `GET /healthz`
- `GET /api/meta`
- `GET /api/chat/sessions`
- `POST /api/chat/sessions`
- `GET /api/chat/sessions/:id/history`
- `POST /api/chat/sessions/:id/turn`
- `GET /api/dashboard/summary`
- `GET /api/dashboard/providers`
- `GET /api/dashboard/tools`
- `GET /api/dashboard/runtime`
- `GET /api/dashboard/config`

后续可选扩展：

- `GET /api/chat/sessions/:id/stream/:turn_id`，通过 SSE
- 更细的 diagnostics 接口
- 本地 Web 资源的安装和更新接口

## 10. 安装与分发模式

同一套协议需要支持多种交付模式。

### 10.1 基础安装

- 只安装 CLI/runtime
- 默认不安装 Web 资源

### 10.2 本地 Web 安装

- 用户在 `onboard` 或后续命令中主动选择
- Web 资源下载或解压到本地目录
- 本地 daemon 提供静态资源，或者用户本地直接打开

建议配置：

```toml
[webchat]
enabled = true
bind = "127.0.0.1:4317"
install_mode = "local_assets"
static_dir = "~/.loongclaw/web/current"
auth_mode = "local_token"
allowed_origins = ["http://127.0.0.1:4317"]
```

### 10.3 托管前端模式

仅作为后续模式：

- 前端可以由 LoongClaw 或用户自行部署到远端
- 前端仍然连接用户自己的本地 runtime
- 需要更严格的鉴权、origin、连接引导和产品说明

该模式明确不是 MVP 优先级。

## 11. 安全模型

MVP 默认策略：

- 默认绑定回环地址
- API 访问需要显式本地 token
- 默认拒绝宽泛 origin
- 不自动开放公网访问

后续托管模式要重点处理：

- 跨域信任模型
- 本地 endpoint 暴露风险说明
- token 生命周期和撤销
- 安全的设备配对 UX
- 明确说明 runtime 仍然运行在本地

## 12. 命令面

建议命令：

- `loongclaw webchat serve`
- `loongclaw webchat status`
- `loongclaw webchat install`
- `loongclaw webchat remove`

`onboard` 可提供：

- `CLI only`
- `CLI + Local Web Console`

安装选项应显示大致的 Web 资源体积。

## 13. 实施计划

### Phase 1：设计与协议

- 固化架构和安装模式
- 定义 API 响应结构
- 定义本地鉴权和 local-only 默认值

### Phase 2：后端复用层

- 从 CLI 现有代码中抽出可复用的 chat session 初始化逻辑
- 在 `crates/app` 中暴露一个对 Web 友好的 conversation service

### Phase 3：本地 HTTP 控制面

- 增加 `webchat serve`
- 实现 chat 和 dashboard 核心接口
- 实现本地 token 鉴权

### Phase 4：前端 MVP

- 实现 chat 页面
- 实现 dashboard 页面
- 建立共享 API client 和连接状态

### Phase 5：可选安装流程

- 将 Web 资源构建与核心 CLI 构建分离
- 增加 install/remove/status 命令
- 接入 `onboard` 选择流程

### Phase 6：托管模式预留

- 保持协议稳定
- 增加 origin 和 endpoint 配置扩展点
- 在本地模式验证成熟前，不推进托管交付

## 14. 待决问题

- 本地静态资源应该由 `loongclaw webchat serve` 直接提供，还是只负责下载资源，
  页面由独立前端开发/生产服务承载？
- Chat 的回复在 MVP 中是普通请求/响应即可，还是首个 PR 就要引入 SSE？
- Dashboard 首版应该只读，还是带有限动作能力？
- Web 资源与本地 daemon 的版本兼容关系应如何约束？

## 15. 首批交付验收标准

首个大型 PR 成功的标准：

- Web Chat 和 Web Dashboard 都存在
- Web 复用了现有 LoongClaw runtime 语义
- 基础安装仍然以 CLI 为主
- Web 资源是可选安装
- 本地部署模式可用且不依赖托管基础设施
- 托管模式仅保留为架构扩展点
