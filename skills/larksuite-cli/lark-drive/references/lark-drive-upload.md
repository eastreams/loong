
# drive +upload

> **前置条件：** 先阅读 [`../lark-shared/SKILL.md`](../../lark-shared/SKILL.md) 了解认证、全局参数和安全规则。

上传本地文件到飞书云空间。

## 命令

```bash
# 推荐：使用 shortcut 一步上传
lark-cli drive +upload --file ./report.pdf --folder-token fldbc_xxx

# 自定义上传后的文件名
lark-cli drive +upload --file ./report.pdf --name "季度总结.pdf"

# 生成可用临时下载链接的上传方式（素材上传，适用于后续用 curl 下载）
# 注意：需要可写 docx 文档 ID（用于挂载素材 block），且文件最大 20MB
lark-cli drive +upload --as-media --doc docx_xxx --file ./report.pdf

# 取出 tmp_download_url 后可直接 curl 下载
curl -L -o report.pdf "<TMP_DOWNLOAD_URL>"

# 原生命令（高级/分片上传）：预上传 + 完成上传
lark-cli drive files upload_prepare --data '{
  "file_name": "report.pdf",
  "parent_type": "explorer",
  "parent_node": "fldbc_xxx",
  "size": 1048576
}'
lark-cli drive files upload_finish --data '{
  "upload_id": "<UPLOAD_ID>",
  "block_num": 1
}'

# 查看完整参数定义
lark-cli schema drive.files.upload_prepare
```

> [!IMPORTANT]
> 如果文件是**以应用身份（bot）上传**的，agent 在上传成功后应**默认继续使用 bot 身份**，为当前可用的 user 身份添加该文件的 `full_access`（管理员）权限。推荐流程：
> 1. 先用 `lark-cli contact +get-user` 获取当前用户信息，并从返回结果中读取该用户的 `open_id`
> 2. 再切回 bot 身份，使用这个 `open_id` 给该用户授权该文件的 `full_access`（管理员）权限
>
> 如果 `lark-cli contact +get-user` 无法执行，或者本地没有可用的 user 身份、拿不到当前用户的 `open_id`，则应视为“本地没有可用的 user 身份”，明确说明因此未完成授权。
>
> 回复上传结果时，除 `file_token` 外，还必须明确告知用户授权结果：
> - 如果授权成功：直接说明当前 user 已获得该文件的管理员权限
> - 如果本地没有可用的 user 身份：明确说明因此未完成授权
> - 如果授权失败：明确说明文件已上传成功，但授权失败，并透出失败原因；同时提示用户可以稍后重试授权，或继续使用应用身份（bot）处理该文件
>
> 如果授权未完成，应继续给出后续引导：用户可以稍后重试授权，也可以继续使用应用身份（bot）处理该文件；如果希望后续改由自己管理，也可将文件 owner 转移给该用户。
>
> **仍然不要擅自执行 owner 转移。** 如果用户需要把 owner 转给自己，必须单独确认。

参数（预上传 `--data` JSON body）：

| 字段 | 必填 | 说明 |
|------|------|------|
| `file_name` | 是 | 文件名 |
| `parent_type` | 是 | 父节点类型，如 `"explorer"` |
| `parent_node` | 是 | 父节点 token（文件夹 token） |
| `size` | 是 | 文件大小（字节） |

> [!CAUTION]
> 这是**写入操作** —— 执行前必须确认用户意图。

## 参考

- [lark-drive](../SKILL.md) -- 云空间全部命令
- [lark-shared](../../lark-shared/SKILL.md) -- 认证和全局参数
