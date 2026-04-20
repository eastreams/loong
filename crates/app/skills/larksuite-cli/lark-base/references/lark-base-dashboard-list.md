# base +dashboard-list

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

分页列出一个 Base 下的仪表盘。

## 推荐命令

```bash
lark-cli base +dashboard-list \
  --base-token VwGhb**************fMnod
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--page-size <n>` | 否 | 每页数量 |
| `--page-token <token>` | 否 | 分页标记 |
| `--format <fmt>` | 否 | 输出格式：json / pretty / table / csv / ndjson |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
GET /open-apis/base/v3/bases/:base_token/dashboards
```

## 返回重点

- 返回 `items / total / page_token / has_more`。
- `items` 仅含 `dashboard_id` 和 `name`。

## 坑点

- `+dashboard-list` 禁止并发调用；批量列多个 Base 时必须串行。

## 参考

- [lark-base-dashboard.md](lark-base-dashboard.md) — dashboard 索引页
