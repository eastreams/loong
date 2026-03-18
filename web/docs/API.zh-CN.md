# LoongClaw Web API 说明

状态：Phase 3 已落地大部分接口，Phase 4 持续扩展  
最后更新：2026-03-18

## 1. 范围

当前 Web API 服务于本地优先的：

- Web Chat
- Web Dashboard

API 的目标是承载前端控制面，而不是重新定义 LoongClaw runtime。

## 2. 基本原则

- 默认绑定回环地址
- 默认需要本地 token 才能访问受保护接口
- 字段命名优先英文
- 前端文案本地化，不由 API 直接下发中文 UI 文字

## 3. 鉴权

当前受保护接口使用：

- `Authorization: Bearer <local-token>`

当前约定：

- `GET /healthz` 可匿名访问
- `GET /api/meta` 可匿名访问
- 其它控制面接口默认需要 token

本地 token 默认文件：

- `~/.loongclaw/web-api-token`

## 4. 响应结构

成功响应：

```json
{
  "ok": true,
  "data": {}
}
```

失败响应：

```json
{
  "ok": false,
  "error": {
    "code": "unauthorized",
    "message": "Token is missing or invalid"
  }
}
```

## 5. 当前已落地接口

### 基础接口

- `GET /healthz`
- `GET /api/meta`

### Chat

- `GET /api/chat/sessions`
- `POST /api/chat/sessions`
- `DELETE /api/chat/sessions/:id`
- `GET /api/chat/sessions/:id/history`
- `POST /api/chat/sessions/:id/turn`
- `GET /api/chat/sessions/:id/turns/:turn_id/stream`

### Dashboard

- `GET /api/dashboard/summary`
- `GET /api/dashboard/providers`
- `GET /api/dashboard/tools`
- `GET /api/dashboard/runtime`
- `GET /api/dashboard/config`
- `GET /api/dashboard/connectivity`

## 6. Chat 流式接口

### `POST /api/chat/sessions/:id/turn`

用途：

- 创建一个新 turn
- 不再同步返回完整 assistant 消息

响应示例：

```json
{
  "ok": true,
  "data": {
    "sessionId": "web-1710000000-ab12cd34",
    "turnId": "turn-1710000001-ff00aa11",
    "status": "accepted"
  }
}
```

### `GET /api/chat/sessions/:id/turns/:turn_id/stream`

用途：

- 读取该 turn 的流式事件

当前协议：

- `fetch` + `Authorization` 请求头
- `application/x-ndjson`
- 一行一个 JSON 事件

当前事件：

- `turn.started`
- `message.delta`
- `tool.started`
- `tool.finished`
- `turn.completed`
- `turn.failed`

事件示例：

```json
{"type":"turn.started","turnId":"turn-1","sessionId":"sess-1","createdAt":"2026-03-18T09:30:00Z"}
{"type":"message.delta","turnId":"turn-1","role":"assistant","delta":"Hello"}
{"type":"message.delta","turnId":"turn-1","role":"assistant","delta":" world"}
{"type":"turn.completed","turnId":"turn-1","message":{"id":"msg-1","role":"assistant","content":"Hello world","createdAt":"2026-03-18T09:30:02Z"}}
```

## 7. 当前流式实现边界

当前流式能力分两层：

1. 前端消费链已经是流式 turn
2. provider 层对 OpenAI-compatible chat completions 会优先尝试真实流式；不支持时退回缓冲路径

当前尚未覆盖：

- 所有 provider 的统一真实流式
- cancel
- reconnect
- resume
- 完整 tool trace

## 8. Dashboard 接口职责

### `GET /api/dashboard/summary`

提供顶部摘要卡数据。

### `GET /api/dashboard/providers`

提供 provider 列表、当前激活项、模型、endpoint 与 key 配置状态。

### `GET /api/dashboard/tools`

提供工具启用状态与策略摘要。

### `GET /api/dashboard/runtime`

提供 runtime 运行态信息，例如：

- config path
- memory mode
- active provider / model
- ingest mode

### `GET /api/dashboard/config`

提供 UI 关心的配置快照，例如：

- endpoint
- API key 是否已配置
- sqlite path
- file root

### `GET /api/dashboard/connectivity`

提供 provider route / connectivity 诊断，例如：

- endpoint
- host
- DNS 结果
- probe 状态
- 是否命中 fake-ip
- 建议的修复方向

响应字段示例：

```json
{
  "ok": true,
  "data": {
    "status": "degraded",
    "endpoint": "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
    "host": "ark.cn-beijing.volces.com",
    "dnsAddresses": ["198.18.0.189"],
    "probeStatus": "transport_failure",
    "probeStatusCode": null,
    "fakeIpDetected": true,
    "proxyEnvDetected": false,
    "recommendation": "direct_host_and_fake_ip_filter"
  }
}
```

## 9. 当前错误表达

当前仍然较常见的 provider 侧错误包括：

- `transport_failure`
- `provider_unavailable`
- `unauthorized`

后续建议继续细化 transport 相关分类，例如：

- DNS / fake-ip 干扰
- connect timeout
- read timeout
- proxy route failure

## 10. 后续 API 方向

当前更合理的下一步是：

1. 继续完善流式文本与 provider 状态事件
2. 继续补 dashboard diagnostics
3. 之后再做 dashboard 受控写入能力
