# Web Notes

## Personalization Prompt State

`daemon` 侧的 personalization 模型目前带有一个 `prompt_state` 字段，可选值包括：

- `pending`
- `configured`
- `deferred`
- `suppressed`

这个字段不是用户偏好本身，而是在表示：

> 后续是否还要继续把 `loong personalize` 作为一个可选的后续引导提示出来。

当前 Web 侧的产品结论：

- 不在 `Abilities -> Personalization` 的编辑表单里暴露 `prompt_state`
- 不在当前 `Abilities` 页面里展示 `deferred / suppressed / pending` 这类流程态文案
- `Personalization` 页面只聚焦真正的操作员偏好：
  - preferred name
  - response density
  - initiative level
  - standing boundaries
  - locale
  - timezone

如果后面 Web 新增专门的 next-steps / advisory 页面，再考虑把 `prompt_state` 放到那类“提示链”界面里，而不是继续塞进主个性化编辑器。

## Channels Follow-up

`Abilities -> Channels` 目前已经具备：

- 左侧摘要
- 右侧渠道列表
- source / readiness / account / service 状态

后续值得继续补的点：

- 区分每个 channel 的 `send` 与 `serve` 能力，而不只是一个笼统 `ready`
- 更明确显示来源：
  - native
  - bridge
  - plugin
  - stub / runtime-backed
- 给 misconfigured channel 增加更具体的原因，而不是只显示计数
- 如果后端 doctor/readiness 继续增强，可以把修复建议接进来，但仍保持只读，而不是先做成管理后台
- 如果内容继续增长，优先在 `Channels` 内做展开详情，不急着拆成单独 `Bridge / Plugin` 页面

当前结论：

- `Channels` 继续作为“渠道接入面板”来做
- 不要过早把它做成完整配置后台
- `bridge / plugin` 更适合作为来源信息出现在这里，而不是先单独成页

## Skills Follow-up

`Abilities -> Skills` 现在的定位应该是：

> 当前有哪些能力，这些能力从哪里来，现在能不能用。

当前已经做了：

- 动态读取 runtime 可见工具列表
- 显示原始 tool id
- 显示来源
- 通过 hover 查看简介

后续值得继续补的点：

- 新 tools 需要继续自动显示，尤其是最近已经出现的：
  - `session_search`
  - `approval_request_*`
  - `delegate_async`
  - `provider.switch`
  - `browser.*`
  - `file.*`
  - `tool.search`
  - `tool.invoke`
- `session_search` 应该作为重点能力被强调，它代表“搜索历史会话内容”，不是普通网页搜索
- 如果后端继续补 catalog/source 关系，可以把来源再做细一点，例如：
  - builtin
  - session
  - browser companion
  - external skill
  - provider
  - delegation
- browser companion 不只显示开关，还应继续显示：
  - 是否 ready
  - command 是否配置
  - 哪些能力依赖它
- external skills 后面可以从摘要继续长成“来源清单”，但仍要保持能力目录感，不要变成另一张状态页

当前结论：

- `Skills` 不只是工具名字列表，而是能力目录
- 后续优先继续接：
  - 新 tools
  - `session_search`
  - source / dependency 关系
- 不要把它做成另一张“状态页”或“插件后台”

## Chat Personalization Follow-up

- 当前本地 personalization 的保存和读取链路是通的：`preferred_name`、`response_density`、`initiative_level` 会写入 `~/.loong/config.toml`，并由 `/api/abilities/personalization` 返回
- 当前剩下的问题是后端 prompt 行为，不是前端保存问题
- 现象：chat 在 personalization 已经存在时，仍可能回答成“我不知道你的偏好称呼”

根因方向：

- personalization 会被渲染成 `## Session Profile`
- 这段 profile 会被注入到 chat 上下文
- 但它目前仍属于 advisory context
- prompt contract 太弱，没能阻止“明明已知却回答不知道”这种自相矛盾回复

最低期望行为：

- 只要 `preferred_name` 已配置，chat 就不应该声称“不知道这个偏好”
- 是否严格服从可以暂时仍保持 advisory，但不能允许这种事实性自相矛盾

## Chat Turn Phase UI Follow-up

`turn.phase` 适合继续接到 Web chat UI，但不应该把后端原始事件名直接暴露给用户。

更合适的方式是把生命周期翻译成轻量、可读的过程提示，例如：

- `preparing` -> 准备上下文
- `requesting_provider` -> 正在请求模型
- `running_tools` -> 正在调用工具
- `requesting_followup_provider` -> 正在整理工具结果
- `finalizing_reply` -> 正在整理最终回答

更推荐的 UI 形态：

- assistant 占位消息上方的小状态条 / chip
- 或输入框上方的一行次级状态文案

主要价值：

- 不再只显示模糊的“生成中”
- 用户能看懂当前是在等模型、跑工具，还是已经接近结束
- chat streaming / tool runtime / session lifecycle 的真实状态可以更自然地被解释出来

## Skills Runtime Truth Follow-up

- `Abilities -> Skills` 需要从“前端静态映射 + 少量名字”升级到“后端 runtime truth 投影”
- 优先接后端已有的 tools catalog / external skills / MCP registry / runtime gating 信息
- 目标不是多加文案，而是让 Web 看到的能力面和后端当前真实能力面一致

## Tool Explainability Follow-up

- `Dashboard / Abilities` 需要更明确解释“为什么可用 / 为什么不可用”
- 不应只停留在 enabled / disabled
- 优先把 workspace、shell cwd、file-root、runtime snapshot、channel readiness 这些后端真值转成可读原因
