# base +dashboard-delete

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

删除仪表盘。

## 推荐命令

```bash
lark-cli base +dashboard-delete \
  --base-token VwGhb**************fMnod \
  --dashboard-id dshxxxxxxx
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
DELETE /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id
```

## 工作流

> [!CAUTION]
> 这是**写入操作**且**不可逆** — 执行前必须向用户确认。

## 坑点

- 删除仪表盘会同时删除其下所有 Block，不可恢复。

## 参考

- [lark-base-dashboard.md](lark-base-dashboard.md) — dashboard 索引页
