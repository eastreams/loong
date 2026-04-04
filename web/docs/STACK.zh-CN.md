# LoongClaw Web 技术栈与目录结构

状态：已进入可用 MVP，持续迭代中  
最后更新：2026-03-22

## 1. 目标

`web/` 目录承载 LoongClaw 的本地优先 Web Console。

当前目标不是独立云端产品，而是基于现有 runtime 提供：

- Web Chat
- Web Status
- Web Abilities
- 首次进入 / onboarding
- 本地诊断与调试入口

## 2. 当前技术栈

前端主栈：

- React 19
- TypeScript 5.9
- Vite 7
- React Router 7
- i18next / react-i18next
- `lucide-react`
- 原生 `fetch` + NDJSON 流读取
- CSS Variables + 自定义主题样式

后端承接：

- `crates/daemon/src/web/mod.rs`
- `crates/daemon/src/web/onboarding.rs`
- `crates/daemon/src/web/auth.rs`
- `crates/daemon/src/web/debug_console.rs`
- Axum 本地 API

## 3. 运行模式

### 开发态

- 前端：Vite dev server
- 后端：本地 daemon API
- 默认地址：
  - `http://127.0.0.1:4173/`
  - `http://127.0.0.1:4317/`

特点：

- 分离前后端
- 热更新快
- 适合日常开发和联调

### 同源产品态骨架

当前已支持：

- daemon 直接托管打包后的静态资源
- 页面和 API 走同一个 origin
- 同源模式下走本地 session cookie，而不是手动 token 主路径

### 安装态

当前已实现第一版安装命令：

- `loongclaw web install --source <dist-dir>`
- `loongclaw web status`
- `loongclaw web remove [--force]`

安装目录：

- `~/.loongclaw/web/dist/`
- 清单：`~/.loongclaw/web/install.json`

## 4. 当前目录

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
      locales/
        en/
        zh-CN/
    components/
      layout/
      status/
      surfaces/
    contexts/
    features/
      abilities/
      chat/
      dashboard/
      onboarding/
    hooks/
    lib/
      api/
      auth/
      config/
      utils/
    styles/
      variables.css
      themes.css
      dashboard.css
      index.css
    main.tsx
```

## 5. 目录职责

### `web/src/features/chat/`

承载：

- session 列表
- history
- turn 创建
- turn 流式读取
- 生成中状态
- 轻量消息渲染

当前关键状态已拆进：

- `hooks/useChatSessions.ts`
- `hooks/useChatStream.ts`

### `web/src/features/abilities/`

当前作为第三个大页面骨架，后续主要承接：

- personalization
- channels snapshot
- skills / external skills

### `web/src/features/dashboard/`

承载：

- runtime 摘要
- tools 摘要
- config 摘要
- connectivity 诊断
- provider 最小写入
- Debug Console

当前关键状态已拆进：

- `hooks/useDashboardData.ts`
- `components/DebugConsolePanel.tsx`

### `web/src/features/onboarding/`

承载：

- onboarding 状态读取
- provider 最小写入
- preferences 轻配置写入
- validate 放行
- token / session 进入流程

当前关键状态已拆进：

- `components/OnboardingStatusPanel.tsx`
- `hooks/useOnboardingFlow.ts`
- `providerConfig.ts`

### `web/src/contexts/` 与 `web/src/hooks/`

当前主要承载：

- Web 会话连接状态
- token / pairing / same-origin session
- onboarding gate

当前关键入口：

- `contexts/WebSessionContext.tsx`
- `hooks/useWebSessionManager.ts`
- `hooks/useWebConnection.ts`

## 6. 当前实现特征

### 路由与页面保活

当前 `chat`、`dashboard` 与 `abilities` 已加入 keep-alive 语义，用来保留切页返回后的可见状态。

收益：

- 流式中切页返回更稳定
- 会话列表与当前可见状态不容易丢

边界：

- 仍需持续关注初始化副作用与页面体积

### 数据访问

当前前端数据访问特点：

- 默认 `credentials: include`
- 开发态可附带本地 token
- 同源产品态优先依赖 session cookie
- Chat 流式消费基于 `fetch + ReadableStream + NDJSON`

### 状态组织

当前以“轻全局 + feature 本地状态”组合为主：

- WebSessionContext：连接 / auth / onboarding gate
- feature hooks：各自页面的数据加载、交互与错误处理

当前尚未引入 Redux / Zustand / React Query 一类额外状态层。

## 7. 脚本与命令

推荐脚本：

- Windows
  - `scripts/web/start-dev.ps1`
  - `scripts/web/stop-dev.ps1`
  - `scripts/web/start-same-origin.ps1`
  - `scripts/web/stop-same-origin.ps1`
- macOS / Linux
  - `scripts/web/start-dev.sh`
  - `scripts/web/stop-dev.sh`
  - `scripts/web/start-same-origin.sh`
  - `scripts/web/stop-same-origin.sh`

## 8. 日志位置

运行日志统一落在用户目录，不再写回仓库：

- `%USERPROFILE%\\.loongclaw\\logs\\web-dev.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-dev.err.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-api.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-api.err.log`

## 9. 专项 review 后的当前重点

这轮 WebUI 专项 review 之后，当前结构上的结论是：

- 大状态机已经开始从页面文件拆到 feature hooks，方向是对的
- Chat / Status / Onboarding 三条主链现在都已有自己的状态 hook
- 近期已修复几条真实运行时问题：
  - 流式失败时不再误删已接受的用户消息
  - 新建会话失败时不再残留空白会话
  - Dashboard 部分成功保存后会回拉真实状态
  - onboarding 不再把 401/session 失效误判成 runtime offline
  - 发送失败后会恢复输入框内容

当前仍值得继续关注：

- `ChatPage.tsx`
- `DashboardPage.tsx`
- `OnboardingStatusPanel.tsx`

这三个页面文件仍偏大，后续继续开发时应优先保持“先抽 hook / 子组件，再加功能”。

## 10. 当前仍未完成

- 更像真实 CLI 的连续输出流 Debug Console
- 更完整的 tool trace / event timeline
- 更完整的 Dashboard 受控写入
- 更顺的安装态产品化体验
- `tool.search` 中文 / 泛化意图召回问题
- Chat 流式的更完整中断 / 重连 / 恢复语义
