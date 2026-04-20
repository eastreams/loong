# base +dashboard-block-create

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。
> **data_config 结构：** 参见 [dashboard-block-data-config.md](dashboard-block-data-config.md) 了解图表类型、通用字段和 filter 规则。

在仪表盘中创建一个 Block（图表组件）。

## 推荐命令

```bash
# 创建柱状图 block
lark-cli base +dashboard-block-create \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --name "订单趋势" \
  --type column \
  --data-config '{"table_name":"订单表","count_all":true,"group_by":[{"field_name":"金额","mode":"integrated"}],"filter":{"conjunction":"and","conditions":[{"field_name":"金额","operator":"isGreater","value":0},{"field_name":"状态","operator":"is","value":"已完成"},{"field_name":"负责人","operator":"isNotEmpty"},{"field_name":"创建日期","operator":"isGreaterEqual","value":1711209600000}]}}'

# 创建指标卡（统计数字字段求和）
lark-cli base +dashboard-block-create \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --name "销售总额" \
  --type statistics \
  --data-config '{"table_name":"数据表","series":[{"field_name":"数字","rollup":"SUM"}]}'

# 创建指标卡（统计记录行数）
lark-cli base +dashboard-block-create \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --name "记录总数" \
  --type statistics \
  --data-config '{"table_name":"数据表","count_all":true}'

# 使用文件传入复杂 data_config
lark-cli base +dashboard-block-create \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --name "销售漏斗" \
  --type funnel \
  --data-config @config.json
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--name <name>` | **是** | Block 名称（允许重名） |
| `--type <type>` | **是** | Block 类型（见 [dashboard-block-data-config.md](dashboard-block-data-config.md) 类型枚举表） |
| `--data-config <json>` | 否 | 数据配置 JSON 对象（支持 `@file.json`） |
| `--user-id-type <type>` | 否 | 用户 ID 类型 |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
POST /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id/blocks
```

**Request Body：**

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | **是** | Block 名称（允许重名） |
| `type` | string | **是** | Block 类型枚举 |
| `data_config` | object | 否 | 数据配置（数据源、维度、指标、筛选等） |

> Create 不支持 `layout`（layout 由后端自动计算）

## 返回重点

| 字段 | 类型 | 说明 |
|------|------|------|
| `block_id` | string | Block ID |
| `name` | string | Block 名称 |
| `type` | string | Block 类型 |
| `layout` | object | 布局信息（只读，后端自动计算） |
| `data_config` | object | 数据配置 |

## 工作流

> [!CAUTION]
> 这是**写入操作** — 执行前必须向用户确认。

1. 先确定图表类型（参见 [dashboard-block-data-config.md](dashboard-block-data-config.md)）。
2. JSON 较大时优先用 `@file.json`。

> [!TIP]
> CLI 会对 `data_config` 做轻量校验与规范化：`series[].rollup` 大写、`group_by[].sort.*` 小写；若需要跳过，使用 `--no-validate`。可直接参考文档尾部的“可复制模板”。

## 坑点

- **`name` 和 `type` 必填** — name 允许重名，type 不可在创建后修改。
- **`layout` 只读** — 由后端自动计算，Create 不支持指定布局。
- **`data_config` 结构随 `type` 变化** — 不同组件类型的字段不同，创建前务必确认类型对应的字段。
- **`count_all` 与 `series` 二选一** — 两者不能同时使用。
- **`user_id_type`** 仅在 filter 涉及人员字段时有意义。

## 参考

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — block 索引页
- [dashboard-block-data-config.md](dashboard-block-data-config.md) — data_config 结构、图表类型、filter 规则
