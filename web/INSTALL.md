# LoongClaw Web Installation Guide

## 1. 前置条件

在运行 Web Console 之前，请先准备：

- Rust 工具链
- Node.js + npm
- 已在仓库根目录构建 `loongclaw` daemon：`cargo build --bin loongclaw`

## 2. 安装前端依赖

进入 `web/` 后执行：

```bash
npm install
```

## 3. 安装模式（推荐）

将构建产物安装到用户目录后直接使用，适合日常体验与非开发场景。

### 构建并安装

```bash
# 在 web/ 目录内
npm run build

# 安装到 ~/.loong/web/（Windows 为 %USERPROFILE%\.loong\web\）
loongclaw web install --source ./dist
```

### 启动

```bash
loongclaw web serve
```

`web serve` 会自动检测已安装的前端，无需额外传参。

默认地址：`http://127.0.0.1:4317/`

### 更新前端

重新构建后再次安装即可，`install` 会直接覆盖已安装内容：

```bash
# 在 web/ 目录内
npm run build
loongclaw web install --source ./dist
```

然后重启 `loongclaw web serve` 生效。

### 管理

```bash
# 查看安装状态（安装时间、来源路径、安装目录）
loongclaw web status

# 卸载前端（保留 daemon）
loongclaw web remove

# 强制卸载（跳过确认）
loongclaw web remove --force
```

安装目录：

- Windows：`%USERPROFILE%\.loong\web\`
- macOS / Linux：`~/.loong/web/`

当前运行态默认也会读取同一根目录下的本地数据文件：
- `config.toml`
- `memory.sqlite3`
- `web-api-token`

## 4. 开发分离模式

适合前端开发、热更新与联调。

### 启动

- Windows：`powershell -File scripts/web/start-dev.ps1`
- macOS / Linux：`bash scripts/web/start-dev.sh`
- 如需强制重编 daemon：Windows 用 `-BuildDaemon`，macOS / Linux 用 `BUILD_DAEMON=1`

### 默认地址

- Web：`http://127.0.0.1:4173/`
- API：`http://127.0.0.1:4317/`

### 停止

- Windows：`powershell -File scripts/web/stop-dev.ps1`
- macOS / Linux：`bash scripts/web/stop-dev.sh`

## 5. 同源静态模式

适合验证更接近产品态的本地入口。daemon 同时托管静态资源和 API，并自动走同源 session。

### 构建前端

```bash
npm run build
```

### 启动

- Windows：`powershell -File scripts/web/start-same-origin.ps1`
- macOS / Linux：`bash scripts/web/start-same-origin.sh`
- 如需强制重编 daemon：Windows 用 `-BuildDaemon`，macOS / Linux 用 `BUILD_DAEMON=1`

默认地址：

- Web UI + API：`http://127.0.0.1:4318/`

### 可选：启动时顺带构建

- Windows：`powershell -File scripts/web/start-same-origin.ps1 -Build`
- macOS / Linux：`BUILD=1 bash scripts/web/start-same-origin.sh`

### 停止

- Windows：`powershell -File scripts/web/stop-same-origin.ps1`
- macOS / Linux：`bash scripts/web/stop-same-origin.sh`

## 6. 环境变量

- `VITE_API_BASE_URL`：开发态显式指定 API 基地址；未设置时，前端对 `4173 → 4317` 做默认映射，同源模式下回退到当前 `origin`

## 7. 日志位置

日志默认写入用户目录：

- Windows：`%USERPROFILE%\.loong\logs\`
- macOS / Linux：`~/.loong/logs/`

常见日志文件：

- `web-dev.log`
- `web-dev.err.log`
- `web-api.log`
- `web-api.err.log`
- `web-same-origin.log`
- `web-same-origin.err.log`
