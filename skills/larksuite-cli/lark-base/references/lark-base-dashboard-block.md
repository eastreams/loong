# base dashboard block shortcuts

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

dashboard block（图表组件）相关命令索引。

## 命令导航

| 文档 | 命令 | 说明 |
|------|------|------|
| [lark-base-dashboard-block-list.md](lark-base-dashboard-block-list.md) | `+dashboard-block-list` | 分页列出仪表盘 Block |
| [lark-base-dashboard-block-get.md](lark-base-dashboard-block-get.md) | `+dashboard-block-get` | 获取 Block 详情 |
| [lark-base-dashboard-block-create.md](lark-base-dashboard-block-create.md) | `+dashboard-block-create` | 创建 Block |
| [lark-base-dashboard-block-update.md](lark-base-dashboard-block-update.md) | `+dashboard-block-update` | 更新 Block |
| [lark-base-dashboard-block-delete.md](lark-base-dashboard-block-delete.md) | `+dashboard-block-delete` | 删除 Block |

## 相关

- [lark-base-dashboard.md](lark-base-dashboard.md) — 仪表盘管理
- [dashboard-block-data-config.md](dashboard-block-data-config.md) — Block data_config 结构、图表类型、filter 规则

## 说明

- 聚合页只保留目录职责；每个命令的详细说明请进入对应单命令文档。
- 所有 `+xxx-list` 调用都必须串行执行；若要批量跑多个 list 请求，只能串行执行。
