# base +record-get

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

获取单条记录，可选裁剪输出字段。

## 推荐命令

```bash
lark-cli base +record-get \
  --base-token app_xxx \
  --table-id tbl_xxx \
  --record-id rec_xxx

lark-cli base +record-get \
  --base-token app_xxx \
  --table-id tbl_xxx \
  --record-id rec_xxx \
  --fields 项目名称,状态
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--table-id <id_or_name>` | 是 | 表 ID 或表名 |
| `--record-id <id>` | 是 | 记录 ID |
| `--fields <csv_or_json>` | 否 | 字段名 CSV，或 JSON 字符串数组 |

## API 入参详情

**HTTP 方法和路径：**

```
GET /open-apis/base/v3/bases/:base_token/tables/:table_id/records/:record_id
```

## 返回重点

- 返回 `record` 和 `raw`。
- `record` 是裁剪后的单条结果；`raw` 保留接口完整响应。

## 参考

- [lark-base-record.md](lark-base-record.md) — record 索引页
