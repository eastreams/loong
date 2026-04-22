# LoongClaw Web

LoongClaw 的本地优先 Web Console。

> 安装与启动说明见 [INSTALL.md](INSTALL.md)。

当前已提供的主要页面与能力：

- `Chat`
- `Dashboard`
- `Abilities`
- `Onboarding`
- Dashboard 内嵌只读 `Debug Console`

当前这版 Web 的重点不是“完整配置后台”，而是把本地 runtime 的真实状态、最小可写配置、聊天流式体验和能力面更稳定地接出来。

## 当前能力概览

- `Chat` 已支持多会话、可见消息历史、流式输出、会话级临时视图态缓存，以及更准确的“生成中”状态提示
- `Chat` 已消费后端 `turn.phase` 事件，并结合 `tool.started / tool.finished` 映射出更真实的前端流式状态
- `Dashboard` 已支持 provider 最小写入、runtime/config/connectivity 读取、工具姿态摘要、审批队列和只读 Debug Console
- `Abilities` 已支持 `Personalization / Channels / Skills / Mascot`
- `Skills` 面板已从前端静态映射，收敛到以后端 runtime/catalog 真值为主
- onboarding 已支持 provider catalog、默认 route 回填、原子 apply-and-validate，以及 kind-route 错配拒绝保存
- 已支持中英文切换与明暗主题切换

## 快速开始

### 方式一：安装态

适合直接使用已经构建好的 Web Console。

1. 构建 daemon：`cargo build --bin loongclaw`
2. 在 `web/` 目录下安装并构建前端：

```bash
npm install
npm run build
```

3. 安装前端产物：

```bash
loongclaw web install --source ./dist
```

4. 启动服务：

```bash
loongclaw web serve
```

默认地址：`http://127.0.0.1:4317/`

管理命令：

```bash
loongclaw web status
loongclaw web remove
```

### 方式二：开发态分离运行

适合前端开发与热更新联调。

1. 构建 daemon：`cargo build --bin loongclaw`
2. 在 `web/` 目录下安装依赖：`npm install`
3. 启动前端 dev server 与本地 API：
   - Windows：`powershell -File scripts/web/start-dev.ps1`
   - macOS / Linux：`bash scripts/web/start-dev.sh`

默认地址：

- Web：`http://127.0.0.1:4173/`
- API：`http://127.0.0.1:4317/`

### 方式三：同源静态模式

适合验证更接近产品态的本地体验。

1. 构建 daemon：`cargo build --bin loongclaw`
2. 在 `web/` 目录下安装依赖：`npm install`
3. 构建前端：`npm run build`
4. 启动同源服务：
   - Windows：`powershell -File scripts/web/start-same-origin.ps1`
   - macOS / Linux：`bash scripts/web/start-same-origin.sh`

默认地址：`http://127.0.0.1:4318/`

## 运行时约定

- Web 默认读取 `~/.loong/` 下的本地配置与状态
- 开发态优先走本地 token / pairing
- 同源产品态优先走本地 session cookie
- Chat 流式当前基于 `HTTP + NDJSON`，不是 WebSocket / SSE

## 相关文档

- [docs/STACK.zh-CN.md](docs/STACK.zh-CN.md)：技术栈、目录结构与运行约定
- [docs/DESIGN.zh-CN.md](docs/DESIGN.zh-CN.md)：当前产品定位、页面分工与设计进度
- [docs/API.zh-CN.md](docs/API.zh-CN.md)：当前 Web 实际依赖的 API 面
- [docs/note.md](docs/note.md)：后续跟进项与专项记录
- [INSTALL.md](INSTALL.md)：安装步骤与常见选项
