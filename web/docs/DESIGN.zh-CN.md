# LoongClaw Web 设计文档

状态：Phase 1 提议  
范围：`alpha-test` 分支，大型交付型 PR  
最后更新：2026-03-17

## 1. 目标

为 LoongClaw 增加一个独立的 `web/` 前端模块，首批至少包含两个主界面：

- Web Chat
- Web Dashboard

Web 只是新的客户端表面，不是新的 assistant runtime。实际执行、会话、provider、tool、memory、audit 语义仍然复用现有 LoongClaw runtime。

## 2. 产品定位

Web 是可选前端模块，不是基础安装的默认组成部分。

- 基础安装仍以 CLI/runtime 为主
- Web 资源默认不进入基础安装包
- `onboard` 可提供 Web 安装选项，并标注体积和用途
- 首批优先支持本地部署模式
- 托管前端模式保留为后续交付，不是 Phase 1 优先级

## 3. 语言与内容策略

前端内容策略改为：

- 优先英文
- 中英双语
- 组织方式与参考站点保持一致

这里的“优先英文”指的是：

- 信息架构先以英文命名和英文路径组织
- 默认翻译资源先提供英文基线
- 中文作为首批同步支持语言，不是后补语言

推荐首批语言：

- `en`
- `zh-CN`

文档本身可以继续以中文维护，但前端产品形态、目录、文案组织、国际化资源结构应与参考站点一致。

## 4. 视觉与交互方向

视觉上允许直接继承参考站点：

- 参考仓库：`E:\GitDesktop\loongclaw-website`
- 可以把它理解成“基于现有官网视觉系统，改造成 chat + dashboard 的产品界面”

这意味着首版不需要刻意做出差异化外观。相反，应尽量继承：

- 颜色 token
- 字体系统
- panel 边框和层次感
- 深浅主题策略
- spacing 和 typography scale
- 动效节奏
- 中英双语切换方式

设计目标不是“做一个新的后台模板”，而是“把现有官网设计语言延展到一个可交互的控制台产品”。

## 5. 核心架构决策

采用 `local-first` 的前后端分离方案。

- 本地 `loongclaw` 进程仍然是真实执行入口和事实来源
- `web/` 是独立前端模块
- 后端暴露本地 HTTP API 作为控制面
- 前端消费本地 API，渲染 chat 和 dashboard

这样可以同时满足：

- Web 独立迭代
- 对无头设备友好
- 将来支持托管前端
- 不破坏现有 CLI / channel / runtime 统一语义

## 6. MVP 非目标

Phase 1 不做这些能力：

- 独立云端 agent runtime
- 多用户服务端模式
- 默认公网暴露
- 远程同步产品闭环
- 托管前端优先交付
- 基础安装默认附带 Web 资源

## 7. 仓库放置与边界

首批继续同仓开发：

```text
crates/
  app/
  daemon/
web/
  docs/
  public/
  src/
  package.json
scripts/
  web/
```

原因：

- 当前协议和 UI 都在快速演进
- 前后端联调和大型 PR 审查在同仓更高效
- 后续如发布节奏明显分离，再考虑拆仓

## 8. 运行时职责划分

### 8.1 后端

`crates/daemon`

- 启动本地 HTTP 服务
- 暴露 Web API
- 处理本地 token 鉴权和 origin 校验
- 可选挂载已安装的本地静态资源

`crates/app`

- 继续承载真正的 conversation runtime
- 提供可复用的 session/turn service
- 负责 provider、tool、memory、ACP、audit 等核心行为

### 8.2 前端

`web/`

- 渲染 Web Chat
- 渲染 Web Dashboard
- 管理主题、语言、连接状态
- 调用后端 API
- 管理本地 token 和实例连接信息

前端不能复制或重写 runtime 逻辑。

## 9. 会话模型

Web 应直接映射到现有 conversation address 模型。

建议：

- `channel_id = web`
- `conversation_id = <browser_session_id>`
- `thread_id = <tab_or_subthread_id>`，后续需要时再引入

浏览器来源、tab、user agent 等信息可以进入 ingress 元数据，但不能承担授权语义。

## 10. 主界面范围

### 10.1 Web Chat

MVP：

- 创建或恢复会话
- 发起一轮对话
- 读取最近历史
- 显示回复状态
- 显示 provider/runtime 简要状态

后续：

- 更完整的 trace
- 附件与工具结果面板
- 多线程会话视图

### 10.2 Web Dashboard

MVP：

- runtime 摘要
- provider 摘要
- memory 摘要
- tool 摘要
- 配置摘要
- doctor/runtime 警告
- Web 安装模式和资源状态

后续：

- 设置页
- Provider Settings 面板，可修改 active provider、model、endpoint、API key
- 运行时管理动作
- 远程实例和多设备视图

### 10.3 Dashboard 配置编辑原则

Web Dashboard 后续可以支持直接管理 provider 配置，但必须遵守以下原则：

- 前端只提供表单和状态反馈
- 真正的配置读取、校验、写入、重载都走后端 API
- 前端不能直接读写 `config.toml`
- API key 不应明文回显，最多显示“已配置”或部分掩码
- 配置变更应具备 `validate -> apply -> reload` 的受控流程
- 配置失败时应允许保留旧值或回滚

建议支持的编辑项：

- active provider
- model
- base URL / endpoint
- API key

不建议在首版开放：

- 任意底层 TOML 编辑
- 未校验直接写入
- 自动公网暴露相关配置

## 11. API 形态

Phase 1 API 维持本地控制面最小集合：

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

后续可扩展：

- SSE 流式回复
- 更细的 diagnostics
- Web 资源安装和更新接口

## 12. 安装与分发

需要同时兼容两种分发方式：

### 12.1 本地部署

- 默认仅安装 CLI/runtime
- 用户在 `onboard` 或后续命令中主动安装 Web 资源
- 本地 daemon 提供静态资源和 API，或前端独立本地运行后连接 API

### 12.2 托管前端

仅作架构预留：

- 前端可由官方或用户自行托管
- 前端仍连接用户自己的本地 runtime
- 需要更严格的 origin、token、endpoint 引导和风险说明

Phase 1 不以该模式为交付重点。

## 13. 安全模型

Phase 1 默认策略：

- 默认仅绑定回环地址
- API 访问需要显式本地 token
- 默认拒绝宽泛 origin
- 不自动开放公网访问

后续托管模式重点补充：

- 跨域信任模型
- token 生命周期和撤销
- endpoint 暴露风险说明
- 设备配对与授权 UX

## 14. 命令面建议

- `loongclaw web serve`
- `loongclaw web status`
- `loongclaw web install`
- `loongclaw web remove`

`onboard` 可提供：

- `CLI only`
- `CLI + Local Web Console`

并显示大致资源体积。

## 15. 实施分期

### Phase 1：协议与设计固化

- 固化前后端边界
- 固化中英双语和参考站点继承策略
- 固化本地安装模式

### Phase 2：后端复用层

- 从 CLI 现有代码中抽可复用的 session/turn 初始化逻辑
- 在 `crates/app` 暴露对 Web 友好的 conversation service

### Phase 3：本地 API 控制面

- 实现 `web serve`
- 实现 chat 和 dashboard 最小接口
- 接入本地 token 鉴权

### Phase 4：前端 MVP

- 基于参考站点结构搭建前端
- 实现双语、主题、布局骨架
- 完成 chat 和 dashboard 两个页面

### Phase 5：可选安装流程

- 分离 Web 构建产物
- 增加 install/remove/status 命令
- 接入 `onboard`

### Phase 6：Dashboard 受控写入能力

- 增加 provider 配置读取和编辑界面
- 增加验证、应用、重载与失败反馈
- 保持 key 掩码显示和安全边界

### Phase 7：托管模式预留

- 保持协议稳定
- 增加 endpoint / origin 扩展点
- 不在本地模式成熟前提前推进托管交付

## 16. 验收标准

首个大型 PR 的验收标准：

- Web Chat 和 Web Dashboard 都存在
- Web 前端复用了现有 LoongClaw runtime 语义
- 基础安装仍以 CLI/runtime 为主
- Web 资源是可选安装
- 本地部署模式可用
- 前端支持 `en` 和 `zh-CN`

## 17. 开发态运行时文件约定

Web 模块的开发态运行日志不应写回仓库目录。

约定：

- 仓库目录 `web/` 只保存源码、文档、静态资源和构建配置
- 开发态后台服务日志统一落到 `%USERPROFILE%\.loongclaw\logs\`

这样可以避免：

- 分支切换时出现大量与源码无关的未跟踪文件
- Git 状态被 `.vite-*.log`、`.web-*.log` 干扰
- 把运行时痕迹错误地放进产品源码树

当前推荐的本地 Web 开发入口：

- `scripts/web/start-dev.ps1`
- `scripts/web/stop-dev.ps1`
- 视觉系统和参考站点保持一致
- 托管模式仅保留架构扩展位，不作为首批依赖
