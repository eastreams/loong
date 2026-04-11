# LoongClaw Web

LoongClaw 的本地优先 Web Console。

> **安装与启动说明请参阅 [INSTALL.md](INSTALL.md)**。

当前已提供：

- `Chat`
- `Status`
- `Abilities`
- `Onboarding`
- `Debug Console`

当前补充进度：

- `Chat` 已支持多会话、本地会话名覆写、最近活跃时间展示与生成中状态提示
- `Abilities` 已接入第一版只读/轻编辑能力页：`Personalization / Channels / Skills`
- 中英文切换
- 明暗主题切换

## 快速开始

### 方式一：安装模式（推荐体验）

适合直接使用已构建的 Web Console，无需运行前端开发服务器：

1. 构建 daemon：`cargo build --bin loongclaw`
2. 安装前端产物（需先构建，见下方）：
   ```bash
   # 在 web/ 目录内
   npm install && npm run build
   # 安装到用户目录
   loongclaw web install --source ./dist
   ```
3. 启动服务：`loongclaw web serve`

默认地址：`http://127.0.0.1:4317/`

管理命令：

```bash
loongclaw web status   # 查看安装状态
loongclaw web remove   # 卸载前端
```

### 方式二：开发分离模式

适合前端开发与热更新联调：

1. 构建 daemon：`cargo build --bin loongclaw`
2. 安装前端依赖：`npm install`（在 `web/` 目录内）
3. 启动前端与本地 API：
   - Windows：`powershell -File scripts/web/start-dev.ps1`
   - macOS / Linux：`bash scripts/web/start-dev.sh`

默认地址：

- Web：`http://127.0.0.1:4173/`
- API：`http://127.0.0.1:4317/`

### 方式三：同源静态模式

适合验证更接近产品态的本地体验（daemon 同时托管静态资源与 API）：

1. 构建 daemon：`cargo build --bin loongclaw`
2. 安装前端依赖：`npm install`
3. 构建前端：`npm run build`
4. 启动同源服务：
   - Windows：`powershell -File scripts/web/start-same-origin.ps1`
   - macOS / Linux：`bash scripts/web/start-same-origin.sh`

默认地址：`http://127.0.0.1:4318/`

当前 Web 运行态默认读取 `~/.loong/`（Windows 为 `%USERPROFILE%\.loong\`）下的
`config.toml`、`memory.sqlite3` 与 `web-api-token`。

## 相关文档

- `docs/STACK.zh-CN.md`：技术栈、目录与运行约定
- `docs/DESIGN.zh-CN.md`：产品形态、onboarding、调试台与专项 review 结论
- `docs/API.zh-CN.md`：当前前端实际依赖的 Web API
- `INSTALL.md`：完整安装步骤与选项
