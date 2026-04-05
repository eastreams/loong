# Web 笔记

## Personalization 的提示状态

daemon 侧的 personalization 模型目前带有一个 `prompt_state` 字段，
可能的值包括：

- `pending`
- `configured`
- `deferred`
- `suppressed`

这个字段**不是用户偏好本身**，而是表示：

> LoongClaw 之后还要不要继续把 `loong personalize` 作为一个可选后续建议提示出来。

当前 Web 端的产品决策：

- **不要**在 `Abilities -> Personalization` 的编辑表单里暴露 `prompt_state`
- **不要**在当前 `Abilities` 页面里展示 `deferred / suppressed / pending` 这类流程状态文案
- Web 的 Personalization 页面只聚焦真正的操作员偏好：
  - preferred name
  - response density
  - initiative level
  - standing boundaries
  - locale
  - timezone

如果后面 Web 新增专门的 next-steps / advisory 页面，再考虑把 `prompt_state`
放到那类“提示链”界面里，而不是继续塞进主个性化编辑器。
