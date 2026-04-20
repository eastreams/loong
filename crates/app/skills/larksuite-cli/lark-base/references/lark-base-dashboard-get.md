# base +dashboard-get

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

获取仪表盘详情，包括主题和组件列表。

## 推荐命令

```bash
lark-cli base +dashboard-get \
  --base-token VwGhb**************fMnod \
  --dashboard-id dshxxxxxxx
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--dashboard-id <id>` | 是 | 仪表盘 ID |
| `--format <fmt>` | 否 | 输出格式 |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

## API 入参详情

**HTTP 方法和路径：**

```
GET /open-apis/base/v3/bases/:base_token/dashboards/:dashboard_id
```

## 返回重点

| 字段 | 类型 | 说明 |
|------|------|------|
| `dashboard_id` | string | 仪表盘 ID |
| `name` | string | 仪表盘名称 |
| `theme` | object | 主题配置 |
| `theme.theme_style` | string | 主题风格：`default` / `SimpleBlue` / `DarkGreen` / `summerBreeze` / `simplistic` / `energetic` / `deepDark` / `futuristic` |
| `blocks` | []object | 组件列表 |
| `blocks[].block_id` | string | 组件 ID |
| `blocks[].block_name` | string | 组件名称 |
| `blocks[].block_type` | string | 组件类型 |

## 参考

- [lark-base-dashboard.md](lark-base-dashboard.md) — dashboard 索引页
- [lark-base-dashboard-block.md](lark-base-dashboard-block.md) — Block 管理
