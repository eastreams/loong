# LoongClaw Web 技术栈与目录结构

状态：已进入可用 MVP，持续迭代中  
最后更新：2026-03-19

## 1. 目标

`web/` 目录承载 LoongClaw 的本地优先 Web Console。当前目标不是做一个独立云端产品，而是基于现有 runtime 补出：
- Web Chat
- Web Dashboard
- 本地开发与联调脚手架

技术与目录组织尽量贴近参考站点 `E:\GitDesktop\loongclaw-website`，同时保持 chat 与 dashboard 的边界清晰。

## 2. 当前技术栈

前端主栈：
- React
- TypeScript
- Vite
- React Router
- i18next
- react-i18next
- CSS Variables + 自定义主题样式
- Lucide React

当前没有引入更重的数据层或 UI kit。请求状态和页面状态目前仍以内建 hooks、context 和 API client 为主。

## 3. 当前目录

```text
web/
  docs/
    API.zh-CN.md
    DESIGN.zh-CN.md
    STACK.zh-CN.md
  public/
  src/
    app/
    assets/
      brand/
      locales/
        en/
        zh-CN/
    components/
      layout/
      status/
      surfaces/
    contexts/
    features/
      chat/
      dashboard/
    hooks/
    lib/
      api/
      auth/
      config/
      utils/
    styles/
      variables.css
      themes.css
      index.css
    main.tsx
  index.html
  package.json
  tsconfig.json
  vite.config.ts
```

## 4. 目录职责

### `web/docs/`

放 Web 相关设计、API 与工程文档。

### `web/public/`

放不参与打包的静态资源。

### `web/src/app/`

应用装配层。负责：
- router
- i18n 初始化
- provider 装配
- 根布局

### `web/src/assets/locales/`

双语文案资源。当前维护：
- `en`
- `zh-CN`

### `web/src/components/`

跨页面共用 UI 组件，例如：
- 顶部导航
- 连接状态
- 面板容器
- 本地 token 提示

### `web/src/contexts/`

全局上下文，目前主要是：
- 主题
- Web 会话连接与鉴权状态

### `web/src/features/chat/`

Chat 的垂直切片，当前包含：
- session 列表
- history 读取
- turn 创建
- turn 流式读取
- 生成中状态展示

### `web/src/features/dashboard/`

Dashboard 的垂直切片，当前包含：
- runtime 摘要
- tools 摘要
- config 摘要
- provider route / connectivity 诊断

### `web/src/lib/`

前端底层能力，不直接承载页面语义。当前主要是：
- API client
- token 存储
- 运行环境配置
- 基础工具函数

### `web/src/styles/`

主题 token 与全局样式系统。
- `variables.css`：设计 token
- `themes.css`：深浅主题映射
- `index.css`：页面与组件样式

## 5. 运行约定

### 开发模式

当前开发模式是：
- 前端：`vite dev`
- 后端：`loongclaw web serve --bind 127.0.0.1:4317`

默认访问：
- 前端：`http://127.0.0.1:4173/`
- 后端：`http://127.0.0.1:4317/`

### 启停脚本

推荐使用：
- `scripts/web/start-dev.ps1`
- `scripts/web/stop-dev.ps1`

### 日志位置

运行时日志不再写回仓库，而是统一落到用户目录：
- `%USERPROFILE%\.loongclaw\logs\web-dev.log`
- `%USERPROFILE%\.loongclaw\logs\web-dev.err.log`
- `%USERPROFILE%\.loongclaw\logs\web-api.log`
- `%USERPROFILE%\.loongclaw\logs\web-api.err.log`

这样可以避免仓库工作区被日志污染，也避免切分支时出现无关未跟踪文件。

## 6. 当前落地情况

前端部分已经不再是静态壳子，当前已具备：
- Chat 与 Dashboard 两个主界面
- 真实后端联调
- 本地 token 鉴权
- Dashboard 读取 `summary / providers / tools / runtime / config / connectivity`
- Chat 流式 turn 消费
- Assistant 下方生成中状态
- 对简单 Markdown 标题、段落和列表的前端渲染

## 7. 近期工程更新

### 流式 turn

前端已切到“两段式 turn”：
- 先创建 turn
- 再读取 NDJSON 流

对于当前 OpenAI-compatible provider 路径，后端会优先尝试真实 provider streaming；不支持时再退回缓冲路径。

### 连接诊断

Dashboard 已新增 provider route / connectivity 诊断能力，用于判断：
- DNS 是否异常
- 是否命中 fake-ip
- provider host 是否可达
- 当前更像是本地路由问题还是上游问题

### 生成中状态

Chat 当前在 assistant 消息下方显示一行生成状态：
- 会根据阶段切换文案
- turn 完成或失败后自动消失
- 不再额外占一个独立气泡

## 8. 当前 Web Onboarding 缺口

从工程角度看，当前 Web 还缺一条完整的首次进入链路。

现在的真实状态是：
- 用户可以先打开前端页面
- 但如果本地实例未准备好，页面仍然只是一个壳子
- 当前仍依赖本地 token 配对
- provider / key / endpoint 等关键配置虽然已开始进入 Web，但 onboarding 仍未形成完整闭环

所以当前 Web 更像“本地实例已准备好后的控制台”，还不是“用户安装后直接开始配置使用的入口”。

## 9. 已落地的 Onboarding 技术落点

### O1：首次进入状态检测

当前已经落地：
- `GET /api/onboard/status`
- `WebSessionContext` 内的 onboarding 状态聚合
- `OnboardingStatusPanel` 首屏状态面板
- onboarding ready 状态下的确认进入

这一层负责在进入主界面前先说明：
- runtime 是否在线
- token 是否已配对
- config 是否存在
- provider 是否已准备好

### O2：最小 provider 可写配置

当前第一版已经落地：
- `POST /api/onboard/provider`
- Web 端最小 provider 配置表单
- Dashboard Provider Settings 接入同一条受控写入链路
- `POST /api/onboard/validate`
- 保存后待验证、验证通过后放行进入 Web

第一版只覆盖：
- provider kind
- model
- base_url / endpoint
- api key

这一层的目标是先补“最小可跑”闭环，不一开始就把 CLI onboard 的全部选项搬进 Web。

## 10. 为后续 Onboarding 预留的技术落点

后续如果要把 Web 做成真正的首次进入入口，建议主要落在这些位置：

### 前端

- `web/src/features/onboarding/`
  - 新增 onboarding 垂直切片
- `web/src/contexts/`
  - 继续复用当前 Web 会话 / 鉴权状态
- `web/src/lib/api/`
  - 新增 onboarding 状态与写入接口

### 后端

- `crates/daemon/src/web_cli.rs`
  - 承接最小 onboarding 接口
  - 保持在 daemon / Web API 层，不急着先做大抽象

### 设计原则

- 首轮只覆盖“能跑起来”的最小配置
- 先解决首次进入路径，再做复杂设置页
- 不把任意 config 写入完全开放给前端

## 10. 当前不优先做的事

当前仍不优先投入：
- 托管模式
- 大型 UI kit 重构
- 复杂 SSR / 多用户服务端模式
- dashboard 全量受控写入的产品化

## 11. 下一步建议

当前更合适的下一步是：
1. 继续打磨 Chat 的流式与错误表达
2. 补上最小 onboarding 状态与配置链路
3. 保持 Dashboard 的诊断与只读配置能力继续完善
