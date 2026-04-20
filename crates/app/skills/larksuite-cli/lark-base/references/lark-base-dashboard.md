# base dashboard shortcuts

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

dashboard 相关命令索引。

## 命令导航

| 文档 | 命令 | 说明 |
|------|------|------|
| [lark-base-dashboard-list.md](lark-base-dashboard-list.md) | `+dashboard-list` | 分页列出仪表盘 |
| [lark-base-dashboard-get.md](lark-base-dashboard-get.md) | `+dashboard-get` | 获取仪表盘详情 |
| [lark-base-dashboard-create.md](lark-base-dashboard-create.md) | `+dashboard-create` | 创建仪表盘 |
| [lark-base-dashboard-update.md](lark-base-dashboard-update.md) | `+dashboard-update` | 更新仪表盘 |
| [lark-base-dashboard-delete.md](lark-base-dashboard-delete.md) | `+dashboard-delete` | 删除仪表盘 |

## 相关

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — 仪表盘 Block（图表组件）管理

## 说明

- 聚合页只保留目录职责；每个命令的详细说明请进入对应单命令文档。
- 所有 `+xxx-list` 调用都必须串行执行；若要批量跑多个 list 请求，只能串行执行。
