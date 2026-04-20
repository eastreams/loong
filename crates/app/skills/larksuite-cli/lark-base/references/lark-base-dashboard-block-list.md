# base +dashboard-block-list

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

分页列出仪表盘中的所有 Block（图表组件）。

## 推荐命令

```bash
lark-cli base +dashboard-block-list \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--page-size <n>` | 否 | 每页数量，默认 20，最大 100 |
| `--page-token <token>` | 否 | 分页标记 |
| `--format <fmt>` | 否 | 输出格式：json / pretty / table / csv / ndjson |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
GET /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id/blocks
```

## 返回重点

| 字段 | 类型 | 说明 |
|------|------|------|
| `items` | []Block | Block 列表 |
| `total` | int | Block 总数 |
| `has_more` | bool | 是否还有更多 |
| `page_token` | string | 下一页分页标记（has_more=true 时返回） |

## 坑点

- `+dashboard-block-list` 禁止并发调用；批量执行时只能串行。

## 参考

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — block 索引页
