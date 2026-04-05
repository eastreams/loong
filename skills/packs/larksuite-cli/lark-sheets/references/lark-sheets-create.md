
# sheets +create（创建表格）

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

本 skill 对应 shortcut：`lark-cli sheets +create`。

特性：

- 一步创建表格并返回 URL
- 可选 `--headers/--data` 在创建后自动写入到第一个工作表的 A1 开始

> [!CAUTION]
> 这是**写入操作** —— 执行前必须确认用户意图。可以先用 `--dry-run` 预览。

> [!IMPORTANT]
> 如果表格是**以应用身份（bot）创建**的，agent 在创建成功后应**默认继续使用 bot 身份**，为当前可用的 user 身份添加该表格的 `full_access`（管理员）权限。推荐流程：
> 1. 先用 `lark-cli contact +get-user` 获取当前用户信息，并从返回结果中读取该用户的 `open_id`
> 2. 再切回 bot 身份，使用这个 `open_id` 给该用户授权该表格的 `full_access`（管理员）权限
>
> 如果 `lark-cli contact +get-user` 无法执行，或者本地没有可用的 user 身份、拿不到当前用户的 `open_id`，则应视为“本地没有可用的 user 身份”，明确说明因此未完成授权。
>
> 回复创建结果时，除 `spreadsheet_token` / `url` 外，还必须明确告知用户授权结果：
> - 如果授权成功：直接说明当前 user 已获得该表格的管理员权限
> - 如果本地没有可用的 user 身份：明确说明因此未完成授权
> - 如果授权失败：明确说明表格已创建成功，但授权失败，并透出失败原因；同时提示用户可以稍后重试授权，或继续使用应用身份（bot）处理该表格
>
> 如果授权未完成，应继续给出后续引导：用户可以稍后重试授权，也可以继续使用应用身份（bot）处理该表格；如果希望后续改由自己管理，也可将表格 owner 转移给该用户。
>
> **仍然不要擅自执行 owner 转移。** 如果用户需要把 owner 转给自己，必须单独确认。

## 命令

```bash
# 最简单：只创建
lark-cli sheets +create --title "仓库管理营收报表"

# 创建并写入表头 + 初始数据
lark-cli sheets +create --title "仓库管理营收报表" \
  --headers '["仓库","统计月份","入库金额","出库金额","销售收入","毛利率"]' \
  --data '[["华东一仓","2026-03",125000,98000,168000,"41.7%"]]'

# 创建到指定文件夹（folder_token）
lark-cli sheets +create --title "测试表" --folder-token "fldbc_xxx"

# 仅预览参数（不发请求）
lark-cli sheets +create --title "测试表" --dry-run
```

## 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `--title <title>` | 是 | 表格标题 |
| `--folder-token <token>` | 否 | 云空间文件夹 token（创建到指定目录） |
| `--headers <json>` | 否 | 一维数组 JSON（表头；写入到 A1） |
| `--data <json>` | 否 | 二维数组 JSON（初始数据；紧跟表头写入） |
| `--dry-run` | 否 | 仅打印参数，不执行请求 |

## 输出

JSON，包含：

- `spreadsheet_token`
- `title`
- `url`

## 参考

- [lark-sheets-write](lark-sheets-write.md) — 后续覆盖写入
- [lark-sheets-append](lark-sheets-append.md) — 后续追加写入
- [lark-shared](../../lark-shared/SKILL.md)
