# base +dashboard-create

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

创建仪表盘。

## 推荐命令

```bash
# 创建仪表盘
lark-cli base +dashboard-create \
  --base-token VwGhb**************fMnod \
  --name "销售报表"

# 创建仪表盘（指定主题）
lark-cli base +dashboard-create \
  --base-token VwGhb**************fMnod \
  --name "销售报表" \
  --theme-style default
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--base-token <token>` | 是 | Base Token |
| `--name <name>` | 是 | 仪表盘名称 |
| `--theme-style <style>` | 否 | 主题风格（见下方枚举） |
| `--dry-run` | 否 | 预览 API 调用，不执行 |

### theme-style 枚举

| 值 | 说明 |
|------|------|
| `default` | 默认主题 |
| `SimpleBlue` | 简约蓝 |
| `DarkGreen` | 深绿 |
| `summerBreeze` | 夏日微风 |
| `simplistic` | 简洁 |
| `energetic` | 活力 |
| `deepDark` | 深色 |
| `futuristic` | 未来感 |

## API 入参详情

**HTTP 方法和路径：**

```
POST /open-apis/base/v3/bases/:base_token/dashboards
```

**Request Body：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 仪表盘名称 |
| `theme` | object | 主题配置 |
| `theme.theme_style` | string | 主题风格 |

## 返回重点

- 返回创建后的仪表盘对象，包含 `dashboard_id`。

## 工作流

> [!CAUTION]
> 这是**写入操作** — 执行前必须向用户确认。

## 坑点

- **dashboard_id** 在 create 返回中取得，后续 get/update/delete 使用。
- **theme_style** 是嵌套在 `theme` 对象下的字段，shortcut 自动包装为 `{"theme": {"theme_style": "..."}}`。

## 参考

- [lark-base-dashboard.md](lark-base-dashboard.md) — dashboard 索引页
