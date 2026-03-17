# LoongClaw Web 目录结构与技术选型

状态：Phase 1 提议  
范围：`web/` 初始结构、技术栈与目录职责  
最后更新：2026-03-17

## 1. 目标

本文回答三个问题：

- `web/` 应该怎么搭
- 首批技术栈怎么选
- 每层目录各自负责什么

当前明确的视觉与工程参考仓库：

- `E:\GitDesktop\loongclaw-website`

Web 模块应尽量沿用该仓库的组织方式、主题方式、中英双语方式和整体视觉语言。

## 2. 总体原则

首批前端采用以下原则：

- 优先英文，中英双语
- 目录结构尽量贴近参考站点
- 外观允许直接复制参考站点的视觉系统
- 在参考站点基础上，把核心页面替换为 `chat + dashboard`
- 先做静态前端，不引入 SSR

换句话说，首版 Web 不需要重新发明设计体系，而是要“复用官网前端的产品化延展版本”。

## 3. Phase 1 推荐技术栈

推荐组合：

- React
- TypeScript
- Vite
- React Router
- i18next
- react-i18next
- CSS Variables
- Lucide React

可选增强：

- TanStack Query
- Zod

## 4. 为什么这么选

### React

- 和参考站点一致
- 足够承载 chat 与 dashboard 的双主界面
- 便于后续做共享组件和布局系统

### TypeScript

- 和参考站点一致
- 更适合约束 API 类型
- 能减少 chat 与 dashboard 共享状态时的漂移

### Vite

- 和参考站点一致
- 本地开发快
- 构建结果天然适合做可选安装的静态资源包

### React Router

- 和参考站点一致
- 足够承载 `/chat`、`/dashboard`、后续 `/settings`

### i18next + react-i18next

- 与参考站点的双语方式一致
- 适合做 `en` / `zh-CN`
- 有利于优先英文、同步中文的资源组织

### CSS Variables

- 与参考站点主题方式一致
- 适合继承颜色 token、字号、间距和深浅主题
- 方便后续继续复用官网风格

### Lucide React

- 和参考站点当前用法一致
- 适合作为 chat / dashboard 的基础图标层

### TanStack Query

不是硬性要求，但建议保留引入空间：

- dashboard 的摘要读取很适合 query cache
- chat 的会话列表、历史刷新也适合统一请求状态

### Zod

不是 Phase 1 必须，但适合：

- 对 API 响应做运行时校验
- 在后端协议快速演进时减少前端静默出错

## 5. Phase 1 不优先的方案

首批不推荐：

- Next.js / SSR
- WebSocket-first 架构
- 重型后台模板框架
- Electron / Tauri
- 大型强绑定 UI kit

原因：

- 当前目标是本地优先的静态前端
- 参考站点本身就是 Vite + SPA 结构
- 协议和信息架构仍在快速变化，先避免过早增加框架复杂度

## 6. 推荐目录方案

建议将 `web/` 搭成下面这样：

```text
web/
  docs/
    API.zh-CN.md
    DESIGN.zh-CN.md
    STACK.zh-CN.md
  public/
    favicon.svg
    site.webmanifest
  src/
    app/
      i18n.ts
      router.tsx
      providers.tsx
    assets/
      locales/
        en/
          common.json
          chat.json
          dashboard.json
        zh-CN/
          common.json
          chat.json
          dashboard.json
    components/
      layout/
      navigation/
      feedback/
      status/
      surfaces/
    contexts/
      ThemeContext.tsx
      WebSessionContext.tsx
    features/
      chat/
        api/
        components/
        hooks/
        pages/
        types/
      dashboard/
        api/
        components/
        hooks/
        pages/
        types/
    hooks/
      useTheme.ts
      useLocale.ts
      useWebConnection.ts
    lib/
      api/
        client.ts
        errors.ts
        types.ts
      auth/
        tokenStore.ts
      config/
        env.ts
      utils/
        format.ts
        storage.ts
    styles/
      variables.css
      index.css
      themes.css
    main.tsx
  index.html
  package.json
  tsconfig.json
  vite.config.ts
```

这个结构的目标不是“炫技式分层”，而是尽量贴近参考站点的组织习惯，同时把 chat 和 dashboard 的边界做清楚。

## 7. 目录用途说明

### `web/docs/`

放 Web 相关开发文档。

- `DESIGN.zh-CN.md`：产品与架构边界
- `API.zh-CN.md`：Phase 1 API 草案
- `STACK.zh-CN.md`：技术选型与目录约定

### `web/public/`

放不会经过打包处理、需要原样输出的静态资源。

适合放：

- favicon
- manifest
- 公开静态图标

### `web/src/app/`

放应用级装配层。

职责：

- 初始化 i18n
- 初始化路由
- 组合全局 provider

这里不放业务细节，只做“把应用拼起来”的事。

### `web/src/assets/locales/`

放双语文案资源。

建议：

- 英文作为主基线
- 中文同步维护
- 按领域拆分为 `common`、`chat`、`dashboard`

这样更接近参考站点的语言组织方式，也方便后续继续扩展。

### `web/src/components/`

放跨页面共享的 UI 组件。

适合放：

- 顶部导航
- 侧栏
- 状态徽标
- 空状态
- 加载态
- 面板容器

这里不放强业务组件，业务组件优先留在 `features/` 内。

### `web/src/contexts/`

放全局 React Context。

建议首批只放两类：

- `ThemeContext`
- `WebSessionContext`

如果某个状态只服务于单个 feature，不要提升到这里。

### `web/src/features/chat/`

放 Chat 相关的完整垂直切片。

职责：

- chat API 封装
- chat 页面
- chat 专属组件
- chat hooks
- chat 类型定义

首批建议把消息流、输入区、会话列表都收在这里。

### `web/src/features/dashboard/`

放 Dashboard 相关的完整垂直切片。

职责：

- dashboard API 封装
- dashboard 页面
- 各类摘要卡片
- health/status hooks
- 后续的 provider settings 表单与校验交互

首批建议把 runtime、provider、memory、tool 的展示组件都收在这里。

### `web/src/hooks/`

放跨 feature 复用的 hooks。

适合放：

- 主题切换
- 语言切换
- Web API 连接状态

不适合放强业务 hooks；强业务 hooks 仍应留在对应 `features/` 下。

### `web/src/lib/`

放不直接属于 UI 的底层前端能力。

`lib/api/`

- HTTP client
- 响应错误处理
- 公共请求类型

`lib/auth/`

- token 读取与存储

`lib/config/`

- 前端运行环境配置
- API base URL 解析

`lib/utils/`

- 纯工具函数

这里要保持“无业务页面语义”，避免变成杂物间。

### `web/src/styles/`

放全局样式系统。

建议与参考站点保持一致的思路：

- `variables.css`：设计 token
- `themes.css`：深浅主题映射
- `index.css`：全局基础样式

Phase 1 应尽量直接复用参考站点的 token 结构和命名方式。

## 8. 页面结构建议

### `/chat`

建议包含：

- 左侧会话列表
- 中间消息区
- 底部输入区
- 顶部连接状态 / provider 状态

### `/dashboard`

建议包含：

- runtime summary
- provider summary
- memory summary
- tools summary
- config summary
- warnings / health

两页应共用同一套 layout、同一套主题和同一套导航语言。

## 9. 与参考站点的一致性要求

首版 Web 应明确继承参考站点以下内容：

- 目录组织思路
- i18n 组织方式
- 主题 token 方式
- 字体组合
- 深浅主题策略
- 面板和边框视觉
- 动效与交互节奏

可以理解为：

“不是参考一下”，而是“在不违背当前功能目标的前提下，尽量直接沿用 `loongclaw-website` 的前端骨架和视觉系统”。

## 10. 未来扩展位

这个目录结构需要为后续留出空间：

- `settings`
- `runtime`
- `providers`
- 托管模式连接向导
- trace / diagnostics

这也是为什么整个模块应命名为 `web`，而不是 `webchat`。

如果后续把 Provider Settings 做大，可以在不打破当前结构的前提下新增：

```text
web/src/features/dashboard/
  settings/
    components/
    hooks/
    forms/
```

这样仍然保持“Dashboard 下的受控配置能力”，而不是把它散落到全局目录中。
