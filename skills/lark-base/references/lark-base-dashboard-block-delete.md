# base +dashboard-block-delete

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

删除仪表盘中的一个 Block。

## 推荐命令

```bash
lark-cli base +dashboard-block-delete \
  --base-token bascn***************CtadY \
  --dashboard-id blkxxx \
  --block-id 9v7g********idcd
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--block-id <id>` | 是 | Block ID |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
DELETE /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id/blocks/:block_id
```

## 工作流

> [!CAUTION]
> 这是**写入操作**且**不可逆** — 执行前必须向用户确认。

## 参考

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — block 索引页
