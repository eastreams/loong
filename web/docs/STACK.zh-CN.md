# LoongClaw Web 技术栈与目录结构

状态：已进入可用 MVP，持续迭代中  
最后更新：2026-03-18

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

放 Web 相关设计、API 和工程文档。

### `web/public/`

放不参与打包的静态资源。

### `web/src/app/`

应用装配层。负责：

- router
- i18n 初始化
- provider 装配
- 根布局

### `web/src/assets/locales/`

双语文案资源。当前已维护：

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

Dashboard 已新增 provider route / connectivity 诊断能力，用于帮助判断：

- DNS 是否异常
- 是否命中 fake-ip
- provider host 是否可达
- 当前更像是本地路由问题还是上游问题

### 生成中状态

Chat 当前在 assistant 消息下方显示一行生成状态：

- 会根据阶段切换文案
- turn 完成或失败后自动消失
- 不再额外占一个独立气泡

## 8. 当前不优先做的事

当前仍不优先投入：

- 托管模式
- 大型 UI kit 重构
- 复杂 SSR / 多用户服务端模式
- dashboard 受控写入的完整产品化

## 9. 下一步建议

当前更合适的下一步是：

1. 继续把 Chat 的流式与错误表达打磨好
2. 补强 provider transport 诊断与 route guidance
3. 继续完善 Dashboard 的 runtime 只读控制台能力
