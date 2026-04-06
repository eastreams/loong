# Web 笔记

## Personalization 的提示状态
daemon 侧的 personalization 模型目前带有一个 `prompt_state` 字段，可能的值包括：

- `pending`
- `configured`
- `deferred`
- `suppressed`

这个字段**不是用户偏好本身**，而是在表示：

> LoongClaw 之后还要不要继续把 `loong personalize` 作为一个可选后续建议提示出来。

当前 Web 端的产品决策：

- **不要**在 `Abilities -> Personalization` 的编辑表单里暴露 `prompt_state`
- **不要**在当前 `Abilities` 页面里展示 `deferred / suppressed / pending` 这类流程状态文案
- Web 的 `Personalization` 页面只聚焦真正的操作员偏好：
  - preferred name
  - response density
  - initiative level
  - standing boundaries
  - locale
  - timezone

如果后面 Web 新增专门的 next-steps / advisory 页面，再考虑把 `prompt_state`
放到那类“提示链”界面里，而不是继续塞进主个性化编辑器。

## Channels 后续方向
`Abilities -> Channels` 目前已经具备：

- 左侧摘要
- 右侧渠道列表
- source / readiness / account / service 状态

后续值得继续补的点：

- 区分每个 channel 的 `send` 与 `serve` 能力，而不是只给一个笼统 ready 状态
- 更明确显示来源：
  - native
  - bridge
  - plugin
  - stub / runtime-backed
- 给 misconfigured 的 channel 增加更具体的原因，而不只是计数
- 如果后端 doctor/readiness 继续长，可以把修复建议接进来，但仍保持只读，不先做成管理后台
- 如果内容继续增长，优先在 `Channels` 里做展开详情，不急着单开 `Bridge / Plugin` 页面

当前结论：

- `Channels` 继续作为“渠道接入面板”来做
- 不要过早把它做成完整配置后台
- `bridge / plugin` 更适合作为来源信息出现在这里，而不是先单独成页

## Skills 后续方向
`Abilities -> Skills` 现在的定位应当是：

> 当前有哪些能力、这些能力从哪来、现在能不能用。

当前已经做了：

- 动态吃 runtime 可见工具列表
- 显示原始 tool id
- 显示来源
- 通过 hover 查看简介

后续值得继续补的点：

- 新 tools 必须继续自动显示，尤其是这轮已经出现的：
  - `session_search`
  - `approval_request_*`
  - `delegate_async`
  - `provider.switch`
  - `browser.*`
  - `file.*`
  - `tool.search`
  - `tool.invoke`
- `session_search` 应该作为重点能力被明确强调，它代表“搜索历史会话内容”，不是普通网页搜索
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
- external skills 后面可以从摘要继续长成“来源清单”，但仍然要保持能力目录感，不要变成另一张状态页

当前结论：

- `Skills` 不只是工具名字列表，而是能力目录
- 后续优先继续接：
  - 新 tools
  - `session_search`
  - source / dependency 关系
- 不要把它做成另一张“状态页”或“插件后台”
