# base +dashboard-block-update

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。
> **data_config 结构：** 参见 [dashboard-block-data-config.md](dashboard-block-data-config.md) 了解图表类型、通用字段和 filter 规则。

更新仪表盘中 Block 的名称或数据配置。

## 推荐命令

```bash
lark-cli base +dashboard-block-update \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --block-id 9v7g********cd \
  --name "订单趋势v2" \
  --data-config '{"table_name":"订单表2","count_all":true,"group_by":[{"field_name":"金额2","mode":"integrated"}]}'
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--block-id <id>` | 是 | Block ID |
| `--name <name>` | 否 | 新名称 |
| `--data-config <json>` | 否 | 数据配置 JSON 对象 |
| `--user-id-type <type>` | 否 | 用户 ID 类型 |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
PATCH /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id/blocks/:block_id
```

**Request Body：**

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 否 | Block 名称 |
| `data_config` | object | 否 | 数据配置 |

> Update 不支持修改 `type` 和 `layout`

## 工作流

> [!CAUTION]
> 这是**写入操作** — 执行前必须向用户确认。

> [!TIP]
> CLI 默认会对 `data_config` 做轻量校验与规范化（见 [dashboard-block-data-config.md](dashboard-block-data-config.md) 的“约束与本地校验”）；如需兼容特殊场景，可加 `--no-validate` 跳过。

## 坑点

- **不可修改 `type` 和 `layout`** — 只能更新 `name` 和 `data_config`。
- **`user_id_type`** 仅在 filter 涉及人员字段时有意义。

## 参考

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — block 索引页
- [dashboard-block-data-config.md](dashboard-block-data-config.md) — data_config 结构详解
