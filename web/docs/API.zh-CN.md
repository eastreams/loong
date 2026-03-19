# LoongClaw Web API 说明

状态：Phase 3 已落地大部分接口，Phase 4 持续扩展  
最后更新：2026-03-19

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
- `GET /api/onboard/status`
- `POST /api/onboard/provider`
- `POST /api/onboard/validate`

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

## 7. 当前流式边界

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
- sliding window

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

当前仍常见的 provider 侧错误包括：
- `transport_failure`
- `provider_unavailable`
- `unauthorized`

后续建议继续细化 transport 相关分类，例如：
- DNS / fake-ip 干扰
- connect timeout
- read timeout
- proxy route failure

## 10. Web Onboarding 接口

### `GET /api/onboard/status`

用途：
- 聚合首次进入所需的只读状态
- 帮前端判断当前卡在哪一步

当前返回范围包括：
- runtime 是否在线
- token 是否必需、是否已配对
- config 是否存在、是否可读
- provider 是否已配置、是否可达
- 当前 active provider / model
- provider base_url / endpoint
- API key 是否已配置
- 当前 blocking stage 与 next action

这一接口允许前端在进入 chat / dashboard 前，先把“为什么现在还不能进入”讲清楚。

### `POST /api/onboard/provider`

用途：
- 受控写入最小 provider 配置

当前请求体：

```json
{
  "kind": "volcengine",
  "model": "doubao-seed-2-0-pro-260215",
  "baseUrlOrEndpoint": "https://ark.cn-beijing.volces.com",
  "apiKey": "..."
}
```

当前第一版写入范围：
- provider kind
- model
- base_url 或显式 endpoint
- api key

当前行为约束：
- 前端不能自由写任意 config 字段
- 后端会在现有 config 基础上做受控更新
- 写入后返回最新 onboarding status，供前端刷新状态
- 当前 onboarding 首屏与 Dashboard `Provider Settings` 共用这条写入接口

当前第一版边界：
- 还没有把 CLI onboard 的轻配置项纳入 Web
- API key 持久化策略后续仍建议继续往 env / secret 路线收敛

### `POST /api/onboard/validate`

用途：
- 对当前最小 provider 配置做一次显式验证
- 用于 onboarding 放行前确认当前 endpoint 与凭证至少通过基础探测

当前返回范围包括：
- `passed`
- `endpointStatus` / `endpointStatusCode`
- `credentialStatus` / `credentialStatusCode`
- `status`（最新的 onboarding status 快照）

当前行为约束：
- 只有 provider 基础配置已存在时才允许验证
- OpenAI-compatible chat providers 会优先做一次最小聊天请求验证
- 其他协议族先走较轻的头部探测
- 前端当前会把“验证通过”作为进入 Web 的显式放行条件之一

## 11. 后续 API 方向

当前更合理的下一步是：
1. 继续完善流式文本与 provider 状态事件
2. 继续补 dashboard diagnostics
3. 补 O2.5 需要的轻配置项接口，例如 personality / memory_profile / system_prompt / addendum
4. 之后再做 dashboard 受控写入能力
