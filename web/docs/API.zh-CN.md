# LoongClaw Web API（当前状态）

本文档只记录当前 Web Console 已落地、且前端正在实际依赖的接口与调用约定。

## 1. 基础接口

### `GET /healthz`

用于确认本地 Web API 是否在线。

### `GET /api/meta`

返回 Web 入口需要的基础元信息。当前前端实际依赖：

- `appVersion`
- `apiVersion`
- `webInstallMode`
- `supportedLocales`
- `defaultLocale`
- `auth.required`
- `auth.scheme`
- `auth.header`
- `auth.tokenPath`
- `auth.tokenEnv`
- `auth.mode`

当前已支持两类鉴权模式：

- `local_token`
- `same_origin_session`

## 2. 认证与客户端调用约定

当前 Web 客户端与本地 API 的交互约定：

- 所有请求默认带 `credentials: include`
- 若浏览器本地保存了 token，请求会额外附带 `Authorization: Bearer <token>`
- `GET /api/meta` 与 `GET /api/onboard/status` 用于入口状态判断
- `same_origin_static` 模式下，前端优先依赖同源 session cookie，而不是手动 token
- 同源写操作保留本地可信 `Origin` 校验

补充说明：

- Chat 流式响应当前基于 `HTTP + NDJSON`
- 当前并未使用 WebSocket / SSE 作为主流式通道

## 3. Onboarding 接口

### `GET /api/onboard/status`

用于首次进入状态聚合。重点字段包括：

- `runtimeOnline`
- `tokenRequired`
- `tokenPaired`
- `configExists`
- `configLoadable`
- `providerConfigured`
- `providerReachable`
- `activeProvider`
- `activeModel`
- `providerBaseUrl`
- `providerEndpoint`
- `providerEndpointExplicit`
- `apiKeyConfigured`
- `personality`
- `memoryProfile`
- `promptAddendum`
- `configPath`
- `blockingStage`
- `nextAction`

常见 `blockingStage`：

- `runtime_offline`
- `token_pairing`
- `session_refresh`
- `missing_config`
- `config_invalid`
- `provider_setup`
- `provider_unreachable`
- `ready`

常见 `nextAction`：

- `start_local_runtime`
- `enter_local_token`
- `refresh_local_session`
- `create_local_config`
- `fix_local_config`
- `configure_provider`
- `validate_provider_route`
- `enter_web`
- `open_chat`

### `POST /api/onboard/provider`

最小 provider 配置写入接口。当前支持：

- `kind`
- `model`
- `baseUrlOrEndpoint`
- `apiKey`

### `POST /api/onboard/provider/apply`

“应用并验证” provider 配置。当前语义：

- 先按候选配置做最小验证
- 仅验证通过时才正式落盘
- 若 `kind` 与 route 明显错配（如标准 `volcengine` 指向 coding plan 路径），会直接返回 `400`
- 返回验证结果与最新 onboarding 状态

### `GET /api/providers/catalog`

提供完整 provider catalog，供 onboarding / dashboard 下拉与默认 route 回填使用。常用字段：

- `kind`
- `displayName`
- `defaultBaseUrl`
- `defaultChatPath`
- `defaultModelsPath`
- `authScheme`
- `featureFamily`
- `isCodingVariant`
- `aliases`
- `configurationHint`

### `POST /api/onboard/preferences`

保存轻配置项。当前支持：

- `personality`
- `memoryProfile`
- `promptAddendum`

### `POST /api/onboard/validate`

执行最小 provider 验证。当前返回重点包括：

- `passed`
- `endpointStatus`
- `endpointStatusCode`
- `credentialStatus`
- `credentialStatusCode`
- `status`

### `POST /api/onboard/pairing/auto`

轻量自动配对接口。当前行为：

- 仅允许本地 loopback 可信来源尝试
- 不把 token 明文返回给前端
- 通过 `HttpOnly` cookie 建立当前浏览器配对状态

### `POST /api/onboard/pairing/clear`

清理当前浏览器自动配对 cookie，用于退出本地配对状态。

## 4. Dashboard 接口

### `GET /api/dashboard/summary`

提供 Dashboard 顶部摘要卡数据。

### `GET /api/dashboard/providers`

提供 provider 列表与当前激活项。常用字段：

- `id`
- `label`
- `enabled`
- `model`
- `endpoint`
- `apiKeyConfigured`
- `apiKeyMasked`
- `defaultForKind`

### `GET /api/dashboard/runtime`

提供 runtime 运行态信息。常用字段：

- `status`
- `source`
- `configPath`
- `memoryBackend`
- `memoryMode`
- `ingestMode`
- `webInstallMode`
- `activeProvider`
- `activeModel`
- `acpEnabled`
- `strictMemory`

### `GET /api/dashboard/config`

提供 UI 关注的配置快照。常用字段：

- `activeProvider`
- `lastProvider`
- `model`
- `providerBaseUrl`
- `providerEndpointExplicit`
- `endpoint`
- `apiKeyConfigured`
- `apiKeyMasked`
- `personality`
- `promptMode`
- `promptAddendumConfigured`
- `memoryProfile`
- `memorySystem`
- `sqlitePath`
- `fileRoot`
- `slidingWindow`
- `summaryMaxChars`

### `GET /api/dashboard/connectivity`

提供 provider route / connectivity 诊断。常用字段：

- `status`
- `endpoint`
- `host`
- `dnsAddresses`
- `probeStatus`
- `probeStatusCode`
- `fakeIpDetected`
- `proxyEnvDetected`
- `recommendation`

### `GET /api/dashboard/tools`

提供工具启用状态与策略摘要。当前前端消费：

- `approvalMode`
- `shellDefaultMode`
- `shellAllowCount`
- `shellDenyCount`
- `items`

当前重点工具项包括：

- `sessions`
- `messages`
- `delegate`
- `browser`
- `browser_companion`
- `web_fetch`
- `web_search`
- `file_tools`
- `external_skills`

### `GET /api/dashboard/debug-console`

提供只读 Debug Console 的块级数据。返回结构：

- `generatedAt`
- `command`
- `blocks`

当前 `blocks` 主要覆盖：

- runtime snapshot
- 最近一次对话 turn
- 最近一次 provider apply / validate
- 最近一次 preferences 保存
- 最近一次 token pairing
- process output

## 5. Chat 接口

### `GET /api/chat/sessions`

读取会话列表。

### `POST /api/chat/sessions`

创建会话。

### `DELETE /api/chat/sessions/{id}`

删除会话。

补充说明：
- 当前没有独立的“重命名会话”后端接口
- Web 里的会话名修改目前是前端本地覆写，不会写回 daemon session 模型

### `GET /api/chat/sessions/{id}/history`

读取会话历史。

当前前端语义：

- 按可见消息计数
- 不让内部 assistant 记录占掉消息泡额度

### `POST /api/chat/sessions/{id}/turn`

创建 turn。当前请求体至少支持：

- `input`
返回：

- `sessionId`
- `turnId`
- `status = accepted`

当前前端约定：

- 一旦 `turn` 被 `accepted`，前端不应再把该轮用户消息整体回滚掉

### `GET /api/chat/sessions/{id}/turns/{turn_id}/stream`

返回 NDJSON 流式事件。当前事件集合：

- `turn.started`
- `message.delta`
- `tool.started`
- `tool.finished`
- `turn.completed`
- `turn.failed`

当前前端消费约定：

- 以换行分隔单位消费 NDJSON
- 保留单行解析失败容错
- `turn.failed` 需要显式反馈到 UI

## 6. 当前边界

当前 API 仍有这些边界：

- Debug Console 还是只读观测面，不是 CLI 镜像
- provider 验证仍是最小验证，不是完整 doctor
- Dashboard 写入仍以最小 provider / preferences 为主
- `tool.search` 的中文 / 泛化意图召回问题仍未解决
- Chat 流式仍缺少更完整的中断 / 重连 / 恢复语义
