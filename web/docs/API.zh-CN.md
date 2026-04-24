# Loong Web API（当前状态）

本文档只记录当前 Web Console 已落地、且前端正在实际依赖的接口与调用约定。

## 1. 基础接口

### `GET /healthz`

用于确认本地 Web API 是否在线。

### `GET /api/meta`

返回 Web 入口需要的基础元信息。当前前端会实际消费：

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

当前支持的认证模式：

- `local_token`
- `same_origin_session`

## 2. 认证与调用约定

当前 Web 客户端与本地 API 的约定如下：

- 所有请求默认带 `credentials: include`
- 如果浏览器里存在本地 token，请求会额外带 `Authorization: Bearer <token>`
- `GET /api/meta` 与 `GET /api/onboard/status` 用于入口状态判断
- same-origin 模式下前端优先依赖本地 session cookie，而不是手动 token
- 同源写操作会做本地可信 `Origin` 校验
- Chat 流式响应当前基于 `HTTP + NDJSON`

## 3. Onboarding 接口

### `GET /api/onboard/status`

用于首次进入时的状态聚合。当前重点字段包括：

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

### `POST /api/onboard/provider`

最小 provider 配置写入接口。当前支持：

- `kind`
- `model`
- `baseUrlOrEndpoint`
- `apiKey`

### `POST /api/onboard/provider/apply`

“应用并验证” provider 配置。当前语义：

- 先对候选配置做最小验证
- 只有验证通过时才正式落盘
- 若 `kind` 与 route 明显错配，会直接拒绝保存并返回明确错误
- 返回最新验证结果与 onboarding 状态

### `GET /api/providers/catalog`

返回 provider catalog，供 onboarding / dashboard 下拉、默认 route 回填与提示文案使用。当前常用字段：

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

执行最小 provider 验证。当前重点返回：

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

清理当前浏览器的自动配对 cookie。

## 4. Abilities 接口

### `GET /api/abilities/personalization`

返回当前个性化摘要。前端实际消费：

- `configured`
- `hasOperatorPreferences`
- `suppressed`
- `promptState`
- `updatedAt`
- `preferredName`
- `responseDensity`
- `initiativeLevel`
- `standingBoundaries`
- `locale`
- `timezone`

### `POST /api/abilities/personalization`

保存基础个性化配置。当前支持：

- `preferredName`
- `responseDensity`
- `initiativeLevel`
- `standingBoundaries`
- `locale`
- `timezone`
- `promptState`

### `GET /api/abilities/channels`

返回 channels snapshot。当前重点字段：

- `catalogChannelCount`
- `configuredChannelCount`
- `configuredAccountCount`
- `enabledAccountCount`
- `misconfiguredAccountCount`
- `runtimeBackedChannelCount`
- `enabledServiceChannelCount`
- `readyServiceChannelCount`
- `surfaces`

其中 `surfaces[]` 当前常用字段包括：

- `id`
- `label`
- `source`
- `configuredAccountCount`
- `enabledAccountCount`
- `misconfiguredAccountCount`
- `readySendAccountCount`
- `readyServeAccountCount`
- `defaultConfiguredAccountId`
- `serviceEnabled`
- `serviceReady`

### `GET /api/abilities/skills`

返回技能/工具能力面的 runtime truth。当前前端实际消费：

- `visibleRuntimeToolCount`
- `visibleRuntimeDirectToolCount`
- `hiddenToolCount`
- `visibleRuntimeTools`
- `visibleRuntimeCatalog`
- `hiddenToolSurfaces`
- `approvalMode`
- `autonomyProfile`
- `consentDefaultMode`
- `sessionsAllowMutation`
- `browserCompanion`
- `externalSkills`

其中：

- `visibleRuntimeCatalog[]` 用于展示真实可见 tool 的名称、summary、execution kind、surface/source 与 usage guidance
- `hiddenToolSurfaces[]` 用于展示隐藏 surface 与其覆盖关系
- `browserCompanion` 用于展示 companion 的 `enabled / ready / commandConfigured / expectedVersion / executionTier / timeoutSeconds`
- `externalSkills` 用于展示 external skills 的 inventory、下载审批、auto expose 与域名约束

## 5. Dashboard 接口

### `GET /api/dashboard/summary`

返回 Dashboard 顶部摘要卡所需数据。

### `GET /api/dashboard/providers`

返回 provider 列表与当前激活项。当前常用字段：

- `id`
- `label`
- `enabled`
- `model`
- `endpoint`
- `apiKeyConfigured`
- `apiKeyMasked`
- `defaultForKind`

### `GET /api/dashboard/runtime`

返回 runtime 运行态信息。当前常用字段：

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

返回 UI 关注的配置快照。当前常用字段：

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
- `promptAddendum`
- `memoryProfile`
- `memorySystem`
- `sqlitePath`
- `fileRoot`
- `slidingWindow`
- `summaryMaxChars`

### `GET /api/dashboard/connectivity`

返回 provider route / connectivity 诊断。当前常用字段：

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

返回工具姿态摘要。当前前端实际消费：

- `approvalMode`
- `autonomyProfile`
- `consentDefaultMode`
- `shellDefaultMode`
- `shellAllowCount`
- `shellDenyCount`
- `sessionsAllowMutation`
- `externalSkillsRequireDownloadApproval`
- `externalSkillsAutoExposeInstalled`
- `externalSkillsBlockedDomainCount`
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

### `GET /api/dashboard/approvals`

返回 Dashboard 的审批队列摘要。当前前端实际消费：

- `pendingApprovalCount`
- `activeApprovalCount`
- `matchedCount`
- `returnedCount`
- `items`

其中 `items[]` 常用字段包括：

- `approvalRequestId`
- `sessionId`
- `sessionTitle`
- `visibleToolName`
- `toolName`
- `status`
- `decision`
- `requestSummary`
- `requestedAt`
- `resolvedAt`
- `executedAt`
- `reason`
- `ruleId`
- `lastError`

### `GET /api/dashboard/debug-console`

返回只读 Debug Console 的分块数据。当前结构：

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

## 6. Chat 接口

### `GET /api/chat/sessions`

读取会话列表。

### `POST /api/chat/sessions`

创建会话。

### `DELETE /api/chat/sessions/{id}`

删除会话。

补充说明：

- 当前没有独立的“重命名会话”后端接口
- Web 里的会话名修改仍是前端本地覆写，不会写回 daemon session 模型

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

- 一旦 turn 被 `accepted`，前端不会再整轮回滚该条用户消息

### `GET /api/chat/sessions/{id}/turns/{turn_id}/stream`

返回 NDJSON 流式事件。当前前端消费的事件集合：

- `turn.started`
- `turn.phase`
- `message.delta`
- `tool.started`
- `tool.finished`
- `turn.completed`
- `turn.failed`

其中 `turn.phase` 当前会携带：

- `phase`
- `providerRound`
- `lane`
- `toolCallCount`
- `messageCount`
- `estimatedTokens`

当前前端消费约定：

- 按换行消费 NDJSON
- 保留单行解析失败容错
- `turn.phase` 会被映射成更轻的 Web `streamPhase`
- `turn.failed` 必须显式反馈到 UI

## 7. 当前边界

当前 API 仍有这些边界：

- Debug Console 仍是只读观测面，不是 CLI 镜像
- provider 验证仍是最小验证，不是完整 doctor
- Dashboard 写入仍以最小 provider / preferences 为主
- 审批队列当前是可解释展示，不是完整 approval 操作台
- Chat 流式仍缺更完整的 cancel / reconnect / resume 语义
