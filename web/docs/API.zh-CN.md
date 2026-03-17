# LoongClaw Web Phase 1 API 草案

状态：Phase 1 草案  
范围：本地优先的 Web Chat + Web Dashboard 控制面 API  
最后更新：2026-03-17

## 1. 目标

Phase 1 API 只服务于本地优先的 Web 控制面：

- Web Chat
- Web Dashboard

API 的目标是稳定承载前端，不重新定义 runtime 语义。

## 2. 总体原则

- 默认仅面向本地实例
- 默认仅绑定回环地址
- 默认需要本地 token
- 响应结构尽量稳定、可扩展
- API 字段命名优先英文
- 用户可见文案不从 API 直接下发，由前端通过双语资源本地化

最后一条很重要：前端采用英文优先、中英双语策略，因此 API 应尽量返回稳定的英文键和值，而不是携带面向 UI 的中文文案。

## 3. 认证方式

Phase 1 推荐：

- `Authorization: Bearer <local-token>`

也可兼容：

- `X-LoongClaw-Token: <local-token>`

首版不引入复杂登录会话。

## 4. 通用响应结构

成功响应：

```json
{
  "ok": true,
  "data": {}
}
```

错误响应：

```json
{
  "ok": false,
  "error": {
    "code": "unauthorized",
    "message": "Token is missing or invalid"
  }
}
```

建议：

- `code` 使用稳定英文标识
- `message` 供调试和日志使用
- 前端可基于 `code` 做双语映射

## 5. 基础接口

### `GET /healthz`

用途：

- 健康检查

示例响应：

```json
{
  "ok": true,
  "data": {
    "status": "ok"
  }
}
```

### `GET /api/meta`

用途：

- 返回运行时和 Web 支撑信息

示例响应：

```json
{
  "ok": true,
  "data": {
    "appVersion": "0.1.0-alpha",
    "apiVersion": "v1",
    "webInstallMode": "local_assets",
    "supportedLocales": ["en", "zh-CN"],
    "defaultLocale": "en"
  }
}
```

## 6. Chat API

### `GET /api/chat/sessions`

用途：

- 获取会话列表

示例响应：

```json
{
  "ok": true,
  "data": {
    "items": [
      {
        "id": "sess_001",
        "title": "Debug memory state",
        "updatedAt": "2026-03-17T10:20:00Z"
      }
    ]
  }
}
```

### `POST /api/chat/sessions`

用途：

- 创建新会话

示例请求：

```json
{
  "title": "New Chat"
}
```

示例响应：

```json
{
  "ok": true,
  "data": {
    "id": "sess_002"
  }
}
```

### `GET /api/chat/sessions/:id/history`

用途：

- 获取会话历史

示例响应：

```json
{
  "ok": true,
  "data": {
    "sessionId": "sess_002",
    "messages": [
      {
        "id": "msg_001",
        "role": "user",
        "content": "Check provider status",
        "createdAt": "2026-03-17T10:25:00Z"
      },
      {
        "id": "msg_002",
        "role": "assistant",
        "content": "Provider is available.",
        "createdAt": "2026-03-17T10:25:03Z"
      }
    ]
  }
}
```

### `POST /api/chat/sessions/:id/turn`

用途：

- 提交一轮用户输入并返回结果

示例请求：

```json
{
  "input": "Summarize the current runtime health"
}
```

示例响应：

```json
{
  "ok": true,
  "data": {
    "turnId": "turn_001",
    "message": {
      "id": "msg_010",
      "role": "assistant",
      "content": "Runtime health is stable."
    }
  }
}
```

后续可扩展：

- SSE 流式输出
- tool trace
- richer message blocks

## 7. Dashboard API

### `GET /api/dashboard/summary`

用途：

- 获取 dashboard 顶层摘要

### `GET /api/dashboard/providers`

用途：

- 获取 provider 状态与可用性

### `GET /api/dashboard/tools`

用途：

- 获取 tool 状态摘要

### `GET /api/dashboard/runtime`

用途：

- 获取 runtime 状态和告警

### `GET /api/dashboard/config`

用途：

- 获取对 UI 有意义的配置摘要

这些接口的字段命名统一使用英文；任何用户可见标题都由前端翻译资源控制。

## 8. Phase 2 Provider Settings API

下面这组接口不属于当前 Phase 1 必做范围，但建议在 Phase 2 作为 Dashboard 的受控配置能力引入。

### `GET /api/dashboard/providers`

用途：

- 获取 provider 列表、当前激活项、模型、endpoint 和 key 配置状态

示例响应：

```json
{
  "ok": true,
  "data": {
    "activeProvider": "openai",
    "items": [
      {
        "id": "openai",
        "label": "OpenAI",
        "enabled": true,
        "model": "gpt-5",
        "endpoint": "https://api.openai.com/v1",
        "apiKeyConfigured": true,
        "apiKeyMasked": "****abcd"
      }
    ]
  }
}
```

### `PATCH /api/dashboard/providers/:id`

用途：

- 更新某个 provider 的可编辑配置

示例请求：

```json
{
  "enabled": true,
  "model": "gpt-5",
  "endpoint": "https://api.openai.com/v1",
  "apiKey": "sk-..."
}
```

说明：

- `apiKey` 只允许写入，不应通过该接口原样回显
- 接口应只接受受控字段，不允许任意配置透传

### `POST /api/dashboard/providers/:id/validate`

用途：

- 对当前编辑值做校验或探测连接

示例响应：

```json
{
  "ok": true,
  "data": {
    "valid": true,
    "warnings": []
  }
}
```

### `POST /api/dashboard/runtime/reload`

用途：

- 在配置写入成功后请求 runtime 重载

示例响应：

```json
{
  "ok": true,
  "data": {
    "reloaded": true
  }
}
```

### Provider Settings 安全约束

- 前端不能直接修改 `config.toml`
- 服务端必须负责校验、持久化和重载
- API key 不应明文回读
- 建议支持失败回滚或保留旧值
- 默认仍然只允许本地受信访问

## 9. 错误码建议

建议首批统一以下英文错误码：

- `unauthorized`
- `forbidden_origin`
- `not_found`
- `invalid_request`
- `runtime_unavailable`
- `provider_unavailable`
- `tool_execution_failed`
- `internal_error`
- `config_validation_failed`
- `runtime_reload_failed`

前端应基于这些 code 做中英双语提示，而不是直接展示服务端 message。

## 10. 兼容性约定

- API path 采用 `/api/...`
- 字段名采用英文 camelCase
- 错误码采用英文 snake_case 或 kebab-case 二选一，首版需固定一种
- `supportedLocales` 至少包含 `en` 和 `zh-CN`

## 11. Phase 1 边界

Phase 1 不要求：

- 复杂登录态
- 远程设备同步
- 多用户权限体系
- 云端会话存储
- 完整托管模式 API
- Provider Settings 写入能力

Phase 1 只需要把本地优先的 Web Chat 与 Web Dashboard 稳定跑通。
