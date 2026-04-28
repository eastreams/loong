# Byted Web Search — 开通与配置

**开箱即用**：注册 → 开通 → 拿 Key → **直接在聊天框把 Key 发给我**（无需编辑配置）→ 完成，后续机器人自动操作。

## 1. 注册

https://www.volcengine.com → 注册（手机/飞书/抖音）→ 实名认证

## 2. 开通

[联网搜索开通](https://console.volcengine.com/search-infinity/web-search) →【正式开通】

计费：约 0.03 元/次。https://www.volcengine.com/docs/85508/1510784

## 3. 获取凭证

**方式 A（推荐）**：[API Key 管理](https://console.volcengine.com/search-infinity/api-key) →【创建 API Key】→ 复制保存

**方式 B（AK/SK）**：控制台头像 → API 访问密钥 → 创建。需配置 `VOLCENGINE_ACCESS_KEY` 和 `VOLCENGINE_SECRET_KEY`。SK 仅显示一次，请及时保存。子账号需授权 `TorchlightApiFullAccess`。

## 4. 配置（把 Key 交给 Claw）

**优先**：拿 Key 后直接在聊天框发给我即可，无需编辑任何配置文件。

**或** 在 Claw 技能/凭证配置中填写 `WEB_SEARCH_API_KEY`：
- **OpenClaw**：编辑 `~/.openclaw/openclaw.json`，在 `skills.entries` 下添加：
  ```json
  "byted-web-search": {
    "enabled": true,
    "env": { "WEB_SEARCH_API_KEY": "您复制的Key" }
  }
  ```
- **其他 Claw**：在技能配置界面填写 `WEB_SEARCH_API_KEY` 即可

**本地使用**：skill 根目录创建 `.env`（内容 `WEB_SEARCH_API_KEY=your_key`），或 `export WEB_SEARCH_API_KEY="..."` 写入 ~/.bashrc。

## 5. 验证

```bash
python3 scripts/web_search.py "北京今日天气"
```

## 常见问题

| 问题 | 解答 |
|------|------|
| SK 忘了 | 无法找回，删除旧密钥重建 |
| 权限错误 | 检查已开通、子账号已授权 TorchlightApiFullAccess |
| 额度用完 | 正式开通后按量计费 |
| 欠费 | 后付费 24h 内充值可恢复 |
| 403 | 检查开通状态与账户 |
| invalid_api_key | 请确认 Key 来自 [联网搜索控制台](https://console.volcengine.com/search-infinity/api-key)（非 Ark），已开通、无空格；Claw 中可重新在聊天框发正确的 Key |
| 429 限流 | 建议单 Key 并发控制在 5 以内，超限降频后重试 |
| 401 InvalidAccessKey | AK/SK 无效或失效，检查密钥或改用 API Key |
| 10400 | 参数错误，检查 Query、Count、TimeRange 等是否符合文档 |
| 10402 | 搜索类型非法，检查 `--type` 是否为 `web` 或 `image` |
| 10403 | 非法账号或无权限，检查账号、Key 或权限配置 |
| 10406 | 免费额度已耗尽，检查账户额度或联系支持 |
| 10407 | 当前无可用免费策略，检查账户状态或联系支持 |
| 10500 | 服务内部错误，建议稍后重试或联系支持 |
| 700429 | 免费链路限流，降频后重试 |
| 100013 | 子账号未授权 TorchlightApiFullAccess |
| Query 超长 | API 规定 1~100 字符，超长可能被截断，建议精简 |

## 链接

开通 https://console.volcengine.com/search-infinity/web-search | API Key https://console.volcengine.com/search-infinity/api-key | API 文档 https://www.volcengine.com/docs/85508/1650263
