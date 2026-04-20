# base +dashboard-block-get

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

获取仪表盘中单个 Block 的详情。

## 推荐命令

```bash
lark-cli base +dashboard-block-get \
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
| `--user-id-type <type>` | 否 | 用户 ID 类型：open_id / union_id / user_id |
| `--format <fmt>` | 否 | 输出格式 |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
GET /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id/blocks/:block_id
```

**Query 参数：**

| 参数 | 必填 | 说明 |
|------|------|------|
| `user_id_type` | 否 | 用户 ID 类型，默认 open_id（仅在 filter 涉及人员字段时使用） |

## 返回重点

| 字段 | 类型 | 说明 |
|------|------|------|
| `block_id` | string | Block ID |
| `name` | string | Block 名称 |
| `type` | string | Block 类型 |
| `layout` | object | 布局信息（只读） |
| `layout.x` | int | X 坐标 |
| `layout.y` | int | Y 坐标 |
| `layout.w` | int | 宽度 |
| `layout.h` | int | 高度 |
| `data_config` | object | 数据配置 |

## 参考

- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — block 索引页
- [dashboard-block-data-config.md](dashboard-block-data-config.md) — data_config 结构详解
