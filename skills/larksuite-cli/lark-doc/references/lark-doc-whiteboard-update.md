
# docs +whiteboard-update（更新飞书画板）

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

更新飞书云文档中的画板内容。这个操作需要提供画板的 Token 和画板的 DSL 内容，并需要使用 whiteboard-cli 工具解析 DSL 内容，并通过管道传入这个命令。
关于如何设计画板内容，以及如何使用 whiteboard-cli，参考 [`../lark-whiteboard/SKILL.md`](../../lark-whiteboard/SKILL.md)。

## 参数

| 参数 | 必填 | 说明                                         |
|------|------|--------------------------------------------|
| `--whiteboard-token` | 是 | 需要更新的画板 token。您需要拥有编辑画板所在文档的权限才能更新画板。      |
| `--idempotent-token` | 否 | 幂等 token，用于确保更新操作是幂等的。默认不填，填写的话最小长度为10个字符。 |
| `--overwrite` | 否 | 覆盖更新画板内容，在更新前删除所有现有内容。默认为 false。           |

## 示例

此处不提供示例调用，请参考 [`../lark-whiteboard/SKILL.md`](../../lark-whiteboard/SKILL.md) 了解完整的使用流程。
