# Byted Web Search 文档索引

火山引擎联网搜索 API 文档，详见官网。

## 核心

| 文档 | 链接 |
|------|------|
| 联网搜索API | https://www.volcengine.com/docs/85508/1650263 |
| 新版 API 参考 | https://www.volcengine.com/docs/87772/2272953 |
| 产品计费 | https://www.volcengine.com/docs/87772/2272951 |
| 产品简介 | https://www.volcengine.com/docs/87772/2272949 |
| 新功能发布记录 | https://www.volcengine.com/docs/87772/2272950 |
| 火山如意数据结构 | https://www.volcengine.com/docs/87772/2272956 |

## 控制台

开通 https://console.volcengine.com/search-infinity/web-search | API Key https://console.volcengine.com/search-infinity/api-key

## 参数速查

| 字段 | 说明 |
|------|------|
| Query | 1~100 字符（超长可能被截断） |
| SearchType | web / image（本 skill 支持）；API 另有 web_summary |
| Count | web 最多 50，image 最多 5 |
| TimeRange | OneDay/OneWeek/OneMonth/OneYear 或 YYYY-MM-DD..YYYY-MM-DD |
| QueryControl.QueryRewrite | --query-rewrite |

## 错误码

| 错误码 | 含义 | 建议处理 |
|------|------|------|
| invalid_api_key | API Key 无效 | 确认 Key 来自联网搜索控制台，非 Ark Key |
| 401 InvalidAccessKey | AK/SK 无效 | 检查 AK/SK 是否正确或已失效 |
| 10400 | 通用参数错误 | 检查 Query、Count、TimeRange 等参数格式 |
| 10402 | 非法搜索类型 | 检查 `SearchType` / `--type` 是否为 `web` 或 `image` |
| 10403 | 非法账号或无权限 | 检查账号、Key 或权限配置 |
| 10406 | 免费额度已耗尽 | 检查账户额度或联系支持 |
| 10407 | 当前无可用免费策略 | 检查账户状态或联系支持 |
| 10500 | 默认内部错误 | 稍后重试，或联系支持 |
| 700429 | 免费限流命中 | 降频后重试 |
| 100013 | 子账号未授权 | 授权 `TorchlightApiFullAccess` |
| 429 / FlowLimitExceeded(100018) | 请求频率过高 | 降频后重试 |

## 相关（凭证不通用）

火山方舟联网搜索（Ark 工具）：https://www.volcengine.com/docs/82379/1756990
