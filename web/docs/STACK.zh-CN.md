# LoongClaw Web 技术栈与目录结构

状态：已进入可用 MVP，持续迭代中  
最后更新：2026-04-22

## 1. 目标

`web/` 目录承载 LoongClaw 的本地优先 Web Console。

当前目标不是独立云端产品，而是基于本地 runtime 提供：

- Web Chat
- Web Dashboard
- Web Abilities
- Onboarding
- 本地调试与状态观察入口

## 2. 当前技术栈

前端主栈：

- React 19
- TypeScript 5.9
- Vite 7
- React Router 7
- i18next / react-i18next
- 原生 `fetch`
- `ReadableStream + NDJSON` 流式读取
- CSS Variables + 主题变量

后端承接：

- `crates/daemon/src/web/serve.rs`
- `crates/daemon/src/web/onboarding.rs`
- `crates/daemon/src/web/abilities.rs`
- `crates/daemon/src/web/dashboard.rs`
- `crates/daemon/src/web/chat.rs`
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

- 前后端分离
- 适合热更新与联调

### 同源产品态

当前已支持：

- daemon 直接托管构建后的静态资源
- 页面与 API 走同一 origin
- 同源模式下优先走本地 session cookie

### 安装态

当前已实现第一版命令：

- `loongclaw web install --source <dist-dir>`
- `loongclaw web status`
- `loongclaw web remove [--force]`

安装目录：

- `~/.loong/web/dist/`
- `~/.loong/web/install.json`

## 4. 当前目录

```text
web/
  docs/
    API.zh-CN.md
    DESIGN.zh-CN.md
    STACK.zh-CN.md
    note.md
  public/
  src/
    app/
    assets/
      locales/
        en/
        zh-CN/
    components/
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
    main.tsx
```

## 5. 目录职责

### `web/src/features/chat/`

承载：

- session 列表
- history
- turn 创建
- turn 流式读取
- tool 状态
- 流式占位消息
- 实验性小宠物显示

关键点：

- `hooks/useChatSessions.ts`
- `hooks/useChatStream.ts`
- `pages/ChatPage.tsx`
- `components/ChatMascot.tsx`
- `mascotPreference.ts`

### `web/src/features/abilities/`

承载：

- personalization 读取与最小编辑
- channels snapshot
- skills runtime truth
- mascot 实验开关

关键点：

- `pages/AbilitiesPage.tsx`
- `components/PersonalizationPanel.tsx`
- `components/ChannelsPanel.tsx`
- `components/SkillsPanel.tsx`
- `components/MascotPanel.tsx`
- `hooks/useAbilitiesData.ts`

### `web/src/features/dashboard/`

承载：

- runtime 摘要
- tools posture 摘要
- config snapshot
- connectivity 诊断
- provider 最小写入
- approval queue
- Debug Console

关键点：

- `pages/DashboardPage.tsx`
- `hooks/useDashboardData.ts`
- `components/DebugConsolePanel.tsx`

### `web/src/features/onboarding/`

承载：

- onboarding 状态读取
- provider 最小写入
- provider apply-and-validate
- preferences 轻配置项写入
- token / session 进入流程

关键点：

- `components/OnboardingStatusPanel.tsx`
- `hooks/useOnboardingFlow.ts`
- `provider/providerConfig.ts`
- `provider/providerCatalog.ts`

### `web/src/contexts/` 与 `web/src/hooks/`

主要承载：

- Web 连接与认证状态
- token / pairing / same-origin session
- onboarding gate

关键入口：

- `contexts/WebSessionContext.tsx`
- `hooks/useWebSessionManager.ts`
- `hooks/useWebConnection.ts`

## 6. 当前实现特征

### 路由与页面保活

当前 `chat / dashboard / abilities` 已加入 keep-alive 语义，用于保留切页返回后的可见状态。

收益：

- 流式中切页返回更稳定
- 会话列表与当前视图态不容易丢

### 数据访问

当前前端数据访问特征：

- 默认 `credentials: include`
- 开发态可附带本地 token
- 同源态优先依赖 session cookie
- Chat 流式基于 `fetch + ReadableStream + NDJSON`

### 状态组织

当前仍采用“轻全局 + feature 本地状态”的组合：

- `WebSessionContext`：连接 / auth / onboarding gate
- feature hooks：每个页面自己的读取、交互与错误处理

当前尚未引入 Redux / Zustand / React Query 这一类额外状态层。

### Runtime truth 接入

这一轮值得单独记住的技术方向：

- `Skills` 已以后端 runtime/catalog 真值为主
- `Dashboard` 已接 autonomy / approval / consent / session mutation 等工具姿态
- `Dashboard` 已显式接 approval queue
- `Chat` 已接 `turn.phase`

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

- `%USERPROFILE%\\.loong\\logs\\web-dev.log`
- `%USERPROFILE%\\.loong\\logs\\web-dev.err.log`
- `%USERPROFILE%\\.loong\\logs\\web-api.log`
- `%USERPROFILE%\\.loong\\logs\\web-api.err.log`

## 9. 当前结构上的判断

当前结构比较明确的结论是：

- 大状态已经开始从页面文件拆到 feature hooks，方向是对的
- Chat / Dashboard / Onboarding / Abilities 都已经形成独立 feature 面
- Debug Console、approval queue、skills runtime truth 这类“读真值”能力适合继续留在现有页面里演进
- 小宠物目前仍应视为 `chat + abilities` 间的实验能力，而不是单独 runtime 子系统

## 10. 当前仍未完成

- 更接近真实 CLI 的连续输出流 Debug Console
- 更完整的 tool trace / event timeline
- 更完整的 Dashboard 受控写入
- 更顺的安装态产品体验
- Chat 流式更完整的中断 / 重连 / 恢复语义
