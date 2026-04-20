# dashboard block data_config 参考

Block 的 `data_config` 字段因 `type` 不同而变化。本文档描述所有共享结构。

## 支持的组件类型（`type` 枚举）

| type 值 | 说明 |
|---------|------|
| `column` | 柱状图 |
| `bar` | 条形图 |
| `line` | 折线图 |
| `pie` | 饼图 |
| `ring` | 环形图 |
| `area` | 面积图 |
| `combo` | 组合图 |
| `scatter` | 散点图 |
| `funnel` | 漏斗图 |
| `wordCloud` | 词云 |
| `radar` | 雷达图 |
| `statistics` | 指标卡 |

## data_config 通用结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `table_name` | string | 关联数据表名称 |
| `series` | `[{ "field_name": "xxx", "rollup": "SUM" }]` | 指标/Y 轴（与 `count_all` 二选一）。rollup 支持 `SUM` / `MAX` / `MIN` / `AVERAGE` |
| `count_all` | boolean | COUNTA 聚合，统计所有记录数（与 `series` 二选一） |
| `group_by` | `[{ "field_name": "xxx", "mode": "integrated" }]` | X 轴分组维度 |
| `filter` | object | 筛选条件 |
| `filter.conjunction` | `"and"` / `"or"` | 筛选逻辑 |
| `filter.conditions` | `[{ "field_name", "operator", "value" }]` | 筛选条件数组，value 类型因字段类型而异（见下方 filter 格式规则） |

## filter 格式规则

**基本结构：**

```json
{
  "filter": {
    "conjunction": "and",
    "conditions": [
      { "field_name": "字段名", "operator": "操作符", "value": "值" }
    ]
  }
}
```

**操作符：**

| 操作符 | 含义 | 是否需要 value |
|--------|------|---------------|
| `is` | 等于 | 是 |
| `isNot` | 不等于 | 是 |
| `contains` | 包含 | 是 |
| `doesNotContain` | 不包含 | 是 |
| `isEmpty` | 为空 | 否 |
| `isNotEmpty` | 不为空 | 否 |
| `isGreater` | 大于 | 是 |
| `isGreaterEqual` | 大于等于 | 是 |
| `isLess` | 小于 | 是 |
| `isLessEqual` | 小于等于 | 是 |

**各字段类型的 value 格式：**

| 字段类型 | value 类型 | 适用操作符 | 示例 |
|----------|-----------|-----------|------|
| 文本 / 电话 / URL | string | is, isNot, contains, doesNotContain, isEmpty, isNotEmpty | `{"field_name":"姓名","operator":"contains","value":"张"}` |
| 数字 | number | is, isNot, isGreater, isGreaterEqual, isLess, isLessEqual, isEmpty, isNotEmpty | `{"field_name":"金额","operator":"isGreater","value":0}` |
| 单选 | string（选项名） | is, isNot, isEmpty, isNotEmpty | `{"field_name":"状态","operator":"is","value":"已完成"}` |
| 多选 | string 或 string[] | is, isNot, contains, doesNotContain, isEmpty, isNotEmpty | `{"field_name":"标签","operator":"contains","value":["紧急","重要"]}` |
| 日期时间 / 创建时间 / 修改时间 | number（毫秒时间戳） | is, isGreater, isGreaterEqual, isLess, isLessEqual, isEmpty, isNotEmpty | `{"field_name":"创建日期","operator":"isGreater","value":1711209600000}` |
| 复选框 | boolean | is | `{"field_name":"已审核","operator":"is","value":true}` |
| 人员 / 创建人 / 修改人 | string 或 string[]（用户 ID） | is, isNot, isEmpty, isNotEmpty | `{"field_name":"负责人","operator":"is","value":"user_id_xxx"}` |
| 所有类型（为空/不为空） | 不需要 value | isEmpty, isNotEmpty | `{"field_name":"备注","operator":"isEmpty"}` |

> `value` 类型为 `string | number | boolean | string[]`，需根据字段类型匹配正确格式

## 约束与本地校验

- 必填与互斥
  - 必填：`table_name`
  - 互斥：`series` 与 `count_all` 二选一，且至少提供其一
- 长度/结构
  - `group_by` 最多 2 个；每项 `field_name` 必填
  - `group_by[].sort.type` 取值 `group|value|view`；`order` 取值 `asc|desc`
- 规范化（CLI 自动处理）
  - `series[].rollup` 自动转成大写（如 `sum` → `SUM`）
  - `group_by[].sort.type/order` 自动转成小写
- 本地校验（可通过 `--no-validate` 跳过）
  - `+dashboard-block-create/update` 默认对 `data_config` 做轻量校验；失败会聚合错误并给出修复建议
  - 仅需传入合法 JSON；CLI 不会擅自改写你的业务含义

## 可复制模板

最小柱状图：

```json
{
  "table_name": "表名",
  "series": [{ "field_name": "数值字段", "rollup": "SUM" }],
  "group_by": [{ "field_name": "分组字段", "mode": "integrated" }]
}
```

最小饼/环图（按分类计数）：

```json
{
  "table_name": "表名",
  "count_all": true,
  "group_by": [{ "field_name": "分类字段", "mode": "integrated" }]
}
```

折线图（按月趋势）：

```json
{
  "table_name": "表名",
  "series": [{ "field_name": "金额", "rollup": "SUM" }],
  "group_by": [{ "field_name": "月份", "mode": "integrated", "sort": {"type":"group","order":"asc"} }]
}
```

## 常见错误与修复

- 同时存在 `series` 与 `count_all`
  - 现象：后端/本地校验报互斥错误
  - 修复：仅保留其一；统计字段用 `series`，统计条数用 `count_all:true`
- 缺少 `table_name`
  - 现象：本地校验缺少必填字段
  - 修复：指定数据源表名（使用表名，非表 ID）
- `series[].rollup` 大小写/取值不合法
  - 现象：本地校验提示枚举不支持
  - 修复：改为 `SUM|MAX|MIN|AVERAGE` 中之一（不区分大小写，CLI 会统一为大写；计数请使用 `count_all:true`）
- `group_by` 超出 2 个或字段名为空
  - 修复：保留前 2 个，或补齐 `field_name`
- 排序枚举不合法
  - 修复：`group_by.sort.type` 仅能为 `group|value|view`；`order` 为 `asc|desc`
- filter 写法不规范
  - 修复：`conjunction` 取 `and|or`；`conditions[].operator` 必须在本页表格列举的范围内；除 `isEmpty/isNotEmpty` 外需提供 `value`

## 指标卡（statistics）data_config 示例

统计数字字段求和：

```json
{
  "table_name": "数据表",
  "series": [{ "field_name": "数字", "rollup": "SUM" }]
}
```

统计记录行数：

```json
{
  "table_name": "数据表",
  "count_all": true
}
```

> `series` 与 `count_all` 二选一，不能同时使用。

## 坑点

- **`count_all` 与 `series` 二选一** — 两者不能同时使用
- **filter `value` 类型因字段而异** — 文本/单选为 string，数字为 number，日期为毫秒时间戳，多选/人员可为 string[]，复选框为 boolean；`isEmpty`/`isNotEmpty` 不需要 value
- **`data_config` 结构随 `type` 变化** — 不同组件类型的字段不同，创建前务必确认类型对应的字段
