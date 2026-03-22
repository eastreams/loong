# LoongClaw Web 技术栈与目录结构

状态：已进入可用 MVP，持续迭代中
最后更新：2026-03-22

## 1. 目标

`web/` 目录承载 LoongClaw 的本地优先 Web Console。

当前目标不是独立云端产品，而是基于现有 runtime 提供：

- Web Chat
- Web Dashboard
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

工程约定：

- 包管理当前以 `npm` 为主（仓库内已有 `package-lock.json`）
- 开发态为 Vite dev server + 本地 daemon API 分离联调
- 当前已支持 `same-origin-static` 本地同源模式
- 产品态方向仍优先收敛到同源托管
- 顶部导航已提供语言切换与明暗主题切换

后端承接：

- `crates/daemon/src/web_cli.rs`（包含 `web install/status/remove/serve` 命令）
- `crates/daemon/src/web_cli/onboarding.rs`
- `crates/daemon/src/web_cli/debug_console.rs`
- Axum 本地 API

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

## 4. 目录职责

### `web/src/features/chat/`

承载：

- session 列表
- history
- turn 创建
- turn 流式读取
- 生成中状态
- 轻量消息渲染

### `web/src/features/dashboard/`

承载：

- runtime 摘要
- tools 摘要
- config 摘要
- connectivity 诊断
- provider 最小写入入口
- Debug Console 入口

### `web/src/features/onboarding/`

承载：

- onboarding 状态读取
- provider 最小写入
- preferences 轻配置写入
- validate 放行
- token pairing 流程

### `web/src/contexts/`

当前主要管理：

- Web 会话连接状态
- token / pairing 状态
- onboarding 状态与放行

### `web/src/styles/`

当前已开始温和拆分：

- `variables.css`：设计 token
- `themes.css`：主题映射
- `dashboard.css`：Dashboard 与 Debug Console
- `index.css`：全局、Chat 与共享布局样式

## 5. 运行约定

开发模式：

- 前端：`vite dev`
- 后端：`loongclaw web serve --bind 127.0.0.1:4317`

默认访问：

- 前端：`http://127.0.0.1:4173/`
- 后端：`http://127.0.0.1:4317/`

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

同源静态模式：

- 默认地址：`http://127.0.0.1:4318/`
- 需要先存在 `web/dist/index.html`（开发用，指定 `--static-root`），或已执行 `loongclaw web install`
- 可由 daemon 直接托管静态资源与 API
- `web serve` 会自动检测 `~/.loongclaw/web/dist/index.html`；检测到后无需传 `--static-root`

安装模式：

- 安装：`loongclaw web install --source <dist-dir>`
- 状态：`loongclaw web status`
- 卸载：`loongclaw web remove [--force]`
- 安装目录：`~/.loongclaw/web/dist/`，清单：`~/.loongclaw/web/install.json`

## 5.5 当前工程实现特征（专项 review 摘要）

这部分补充“代码当前实际上是怎么组织和运行的”，便于后续 WebUI 改造时快速建立共同认知。

### 路由装配

当前 `chat` 和 `dashboard` 还不是严格意义上的“按路由挂载”，而是：

- 由 `app/router.tsx` 同时挂载 `ChatPage` 与 `DashboardPage`
- 再通过 `hidden` / `aria-hidden` 切换可见性

这意味着：

- 两个页面的初始化副作用会同时存在
- 页面切换更接近“单页分区切换”，而不是标准路由卸载/挂载

### 状态组织

当前状态管理是“轻全局 + 页面本地状态”为主：

- `WebSessionContext` 负责 endpoint、auth、token、pairing、onboarding gate
- `ChatPage` / `DashboardPage` / `OnboardingStatusPanel` 主要通过 `useState + useEffect` 管理各自状态
- 当前未引入 Redux / Zustand / React Query 一类额外状态层

### 数据访问

当前前端请求模式主要是：

- 所有请求默认 `credentials: include`
- 本地存在 token 时，额外附带 `Authorization: Bearer <token>`
- Chat 流式响应通过 `fetch + ReadableStream + NDJSON` 消费
- `meta / onboard status` 与 feature API 目前还存在两套调用风格

### 样式组织

当前样式体系以全局 CSS 为主：

- `variables.css` 负责设计 token
- `themes.css` 负责主题映射
- `dashboard.css` 负责 Dashboard / Debug Console
- `index.css` 仍承担全局布局、Chat 与共享样式

## 6. 日志位置

运行日志统一落在用户目录，不再写回仓库：

- `%USERPROFILE%\\.loongclaw\\logs\\web-dev.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-dev.err.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-api.log`
- `%USERPROFILE%\\.loongclaw\\logs\\web-api.err.log`

## 7. 当前已落地链路

### O1：首次进入状态检测

已落地：

- `GET /api/onboard/status`
- 首屏状态面板
- ready 状态确认进入

### O2：最小 provider 可写配置

已落地：

- `POST /api/onboard/provider`
- onboarding 表单
- Dashboard `Provider Settings`

当前支持字段：

- provider kind
- model
- base_url / endpoint
- api key

### O3：验证与放行

已落地：

- `POST /api/onboard/validate`
- `POST /api/onboard/provider/apply`
- 应用后在当前页验证，而不是强制回 onboarding

### O4：token / pairing 收口

已部分落地：

- token 输入已进入 onboarding
- 顶部散落 token banner 已移除
- 轻自动配对 + 手动兜底

### O2.5：轻配置项

已部分落地：

- `personality`
- `memory_profile`
- `prompt addendum`

当前优先落在 onboarding 首屏的“可选个性化设置”。

### Debug Console

已落地第一版：

- Dashboard 内嵌切换
- 只读
- 终端风格
- 以“操作块”形式展示最近事件

## 8. 当前仍未完成

- 完整的 Dashboard 轻配置项写入闭环
- 更像真实 CLI 的连续输出流
- 更完整的 tool trace / event timeline
- 更平滑的安装态打包分发体验（`web install` 已落地，分发产物的打包流程尚未建立）
- 安装态级别的无感 token 配对（命令已实现，session 自动绑定链路待完善）
- 更强的 provider / tool doctor

## 9. 近期值得关注的新事项

这段时间新增且会影响后续开发的事项：

- Debug Console 已从”卡片拼接”转向”操作块分段”展示
- Chat 历史已改成按**可见消息**计数
- Dashboard 工具区已对齐更多 runtime 能力：
  - `web_search`
  - `browser_companion`
  - `file_tools`
- macOS 启停脚本已补齐
- provider apply 已收成当前页验证流程
- **`web install/status/remove` 命令已落地**，安装流程：
  1. `npm run build`（或 CI 产出 `dist/`）
  2. `loongclaw web install --source ./dist`
  3. `loongclaw web serve`（自动检测已安装前端，无需 `--static-root`）

## 10. 接下来最适合继续做什么

结合 2026-03-21 的专项 review，推荐顺序调整为：

1. 先把 `chat / dashboard` 改成真实路由挂载，避免双页面同时初始化与重复请求
2. 统一 Web 数据层，把 auth / onboarding / feature API 的请求、401、错误与取消机制收口
3. 优先拆分 `ChatPage`、`DashboardPage`、`WebSessionContext` 这几个大文件
4. 补齐运行时校验、流式解析容错、超时与中断能力
5. 逐步推动同源 session 收口，降低前端对本地 token 持久化的依赖
6. 在上述骨架稳定后，再继续扩展 Debug Console、Dashboard 轻配置与产品态能力
