# Copyright (c) 2025 Beijing Volcano Engine Technology Co., Ltd. and/or its affiliates.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

#!/usr/bin/env python3
"""火山引擎联网搜索 API 客户端。

官方文档：https://www.volcengine.com/docs/85508/1650263
签名参考：https://github.com/volcengine/volc-openapi-demos/blob/main/signature/python/sign.py

凭证（Claw 中优先）：拿 Key 后直接在聊天框发给我即可，无需编辑配置。
认证优先级：1) WEB_SEARCH_API_KEY 或 --api-key  2) VOLCENGINE_ACCESS_KEY+SECRET_KEY  3) VeFaaS IAM

示例：
    python web_search.py "北京天气"
    python web_search.py "OpenAI 最新发布" --time-range OneWeek
    python web_search.py "故宫博物院" --type image --count 3
"""

import argparse
import datetime
import getpass
import hashlib
import hmac
import json
import os
import re
import shlex
import sys
from pathlib import Path
from typing import Optional
from urllib.parse import quote

SERVICE = "volc_torchlight_api"
VERSION = "2025-01-01"
REGION = "cn-beijing"
HOST = "mercury.volcengineapi.com"
ACTION = "WebSearch"
INTERNAL_API_URL = "https://open.feedcoopapi.com/search_api/web_search"
TRAFFIC_TAG_HEADER = "X-Traffic-Tag"
TRAFFIC_TAG_VALUE = "skill_web_search_common"
TIME_RANGE_SHORTCUTS = {"OneDay", "OneWeek", "OneMonth", "OneYear"}
DATE_RANGE_PATTERN = re.compile(r"^(\d{4}-\d{2}-\d{2})\.\.(\d{4}-\d{2}-\d{2})$")
LEGACY_ENV_PATH = "/root/.openclaw/.env"
USER_ENV_PATH = str(Path.home() / ".openclaw/.env")
SUMMARY_PREVIEW_LIMIT = 1000
ERROR_HINTS = {
    "10400": "提示：参数错误。请检查 Query、Count、TimeRange 等参数格式是否正确。",
    "10402": "提示：搜索类型非法。当前仅支持 web 或 image。",
    "10403": "提示：账号或权限异常。请确认 API Key 来自联网搜索控制台，或检查账号权限。",
    "10406": "提示：免费额度已耗尽。请检查账户额度或联系支持。",
    "10407": "提示：当前无可用免费策略。请检查账户状态或联系支持。",
    "10500": "提示：服务内部错误。建议稍后重试，或联系支持。",
    "700429": "提示：免费链路触发限流。请降频后重试。",
    "100013": "提示：子账号未授权 TorchlightApiFullAccess。",
}


# ---- 依赖加载 ----

def _require_requests():
    try:
        import requests
    except ImportError:
        print("Error: requests not installed. Run: pip install requests", file=sys.stderr)
        sys.exit(1)
    return requests


def _load_legacy_env_file(env_path: str = LEGACY_ENV_PATH) -> None:
    if not os.path.exists(env_path):
        return

    try:
        with open(env_path, "r", encoding="utf-8") as f:
            for raw_line in f:
                line = raw_line.strip()
                if not line or line.startswith("#"):
                    continue
                if line.startswith("export "):
                    line = line[len("export "):].strip()
                if "=" not in line:
                    continue

                key, value = line.split("=", 1)
                key = key.strip()
                value = value.strip()
                if not key:
                    continue

                try:
                    parsed = shlex.split(value, comments=True)
                    value = parsed[0] if parsed else ""
                except ValueError:
                    value = value.strip("\"'")

                os.environ.setdefault(key, value)
    except OSError:
        return


def _load_legacy_env_files() -> None:
    seen_paths = set()
    for env_path in (LEGACY_ENV_PATH, USER_ENV_PATH):
        normalized = os.path.abspath(os.path.expanduser(env_path))
        if normalized in seen_paths:
            continue
        seen_paths.add(normalized)
        _load_legacy_env_file(normalized)


# ---- 火山引擎 HMAC-SHA256 签名 (基于官方示例) ----

def _hmac_sha256(key: bytes, content: str) -> bytes:
    return hmac.new(key, content.encode("utf-8"), hashlib.sha256).digest()


def _hash_sha256(content: str) -> str:
    return hashlib.sha256(content.encode("utf-8")).hexdigest()


def _norm_query(params: dict) -> str:
    query = ""
    for key in sorted(params.keys()):
        if isinstance(params[key], list):
            for value in params[key]:
                query += quote(key, safe="-_.~") + "=" + quote(value, safe="-_.~") + "&"
        else:
            query += quote(key, safe="-_.~") + "=" + quote(str(params[key]), safe="-_.~") + "&"
    return query[:-1].replace("+", "%20") if query else ""


def _utc_now():
    try:
        from datetime import timezone
        return datetime.datetime.now(timezone.utc)
    except ImportError:
        return datetime.datetime.utcnow()


def _sign_request(method: str, ak: str, sk: str, body: str, session_token: str = "") -> dict:
    now = _utc_now()
    x_date = now.strftime("%Y%m%dT%H%M%SZ")
    short_date = x_date[:8]
    x_content_sha256 = _hash_sha256(body)
    content_type = "application/json"

    query_params = {"Action": ACTION, "Version": VERSION}

    signed_header_keys = ["content-type", "host", "x-content-sha256", "x-date", "x-traffic-tag"]
    if session_token:
        signed_header_keys.append("x-security-token")
    signed_header_keys.sort()
    signed_headers_str = ";".join(signed_header_keys)

    canonical_header_lines = [
        f"content-type:{content_type}",
        f"host:{HOST}",
        f"x-content-sha256:{x_content_sha256}",
        f"x-date:{x_date}",
        f"x-traffic-tag:{TRAFFIC_TAG_VALUE}",
    ]
    if session_token:
        canonical_header_lines.append(f"x-security-token:{session_token}")
        canonical_header_lines.sort()

    canonical_request = "\n".join(
        [
            method.upper(),
            "/",
            _norm_query(query_params),
            "\n".join(canonical_header_lines),
            "",
            signed_headers_str,
            x_content_sha256,
        ]
    )

    credential_scope = f"{short_date}/{REGION}/{SERVICE}/request"
    string_to_sign = "\n".join(
        [
            "HMAC-SHA256",
            x_date,
            credential_scope,
            _hash_sha256(canonical_request),
        ]
    )

    k_date = _hmac_sha256(sk.encode("utf-8"), short_date)
    k_region = _hmac_sha256(k_date, REGION)
    k_service = _hmac_sha256(k_region, SERVICE)
    k_signing = _hmac_sha256(k_service, "request")
    signature = _hmac_sha256(k_signing, string_to_sign).hex()

    authorization = (
        f"HMAC-SHA256 Credential={ak}/{credential_scope}, "
        f"SignedHeaders={signed_headers_str}, "
        f"Signature={signature}"
    )

    headers = {
        "Content-Type": content_type,
        "Host": HOST,
        "X-Date": x_date,
        "X-Content-Sha256": x_content_sha256,
        TRAFFIC_TAG_HEADER: TRAFFIC_TAG_VALUE,
        "Authorization": authorization,
    }
    if session_token:
        headers["X-Security-Token"] = session_token
    return headers


# ---- 凭证获取 ----

def _get_credentials() -> tuple:
    """返回 (ak, sk, session_token)。"""
    ak = os.getenv("VOLCENGINE_ACCESS_KEY")
    sk = os.getenv("VOLCENGINE_SECRET_KEY")
    if ak and sk:
        return ak, sk, ""

    try:
        from veadk.auth.veauth.utils import get_credential_from_vefaas_iam

        cred = get_credential_from_vefaas_iam()
        return cred.access_key_id, cred.secret_access_key, cred.session_token
    except Exception:
        return None, None, ""


# ---- 请求构建 ----

def _get_api_key(cli_api_key: Optional[str]) -> Optional[str]:
    api_key = cli_api_key or os.getenv("WEB_SEARCH_API_KEY")
    return api_key.strip() if api_key else None


def _validate_time_range(time_range: Optional[str]) -> Optional[str]:
    if not time_range:
        return None
    if time_range in TIME_RANGE_SHORTCUTS:
        return time_range

    match = DATE_RANGE_PATTERN.match(time_range)
    if not match:
        raise ValueError(
            "--time-range 需为 OneDay/OneWeek/OneMonth/OneYear，或日期区间 YYYY-MM-DD..YYYY-MM-DD。"
        )

    start_text, end_text = match.groups()
    try:
        start_date = datetime.date.fromisoformat(start_text)
        end_date = datetime.date.fromisoformat(end_text)
    except ValueError as exc:
        raise ValueError("--time-range 中的日期需为有效的 YYYY-MM-DD。") from exc

    if start_date > end_date:
        raise ValueError("--time-range 的开始日期不能晚于结束日期。")

    return time_range


def build_body(
        query: str,
        search_type: str = "web",
        count: int = 5,
        time_range: Optional[str] = None,
        auth_level: int = 0,
        query_rewrite: bool = False,
) -> dict:
    body = {"Query": query, "SearchType": search_type, "Count": count}

    if search_type == "web":
        body["NeedSummary"] = True
        filters = {}
        if auth_level > 0:
            filters["AuthInfoLevel"] = auth_level
        if filters:
            body["Filter"] = filters
        if time_range:
            body["TimeRange"] = time_range

    if query_rewrite:
        body["QueryControl"] = {"QueryRewrite": True}

    return body


# ---- API 调用 ----

def do_search(
        body: dict,
        api_key: Optional[str] = None,
        ak: Optional[str] = None,
        sk: Optional[str] = None,
        session_token: str = "",
):
    requests = _require_requests()
    body_str = json.dumps(body, ensure_ascii=False)
    if api_key:
        headers = {
            "Content-Type": "application/json",
            TRAFFIC_TAG_HEADER: TRAFFIC_TAG_VALUE,
            "Authorization": f"Bearer {api_key}",
        }
        url = INTERNAL_API_URL
    else:
        if not ak or not sk:
            raise ValueError("missing volcengine credentials")
        headers = _sign_request("POST", ak, sk, body_str, session_token)
        url = f"https://{HOST}?Action={ACTION}&Version={VERSION}"

    response = requests.post(url, headers=headers, data=body_str.encode("utf-8"), timeout=30)
    response.raise_for_status()
    return response.json()


# ---- 输出格式化 ----

def format_output(data: dict, search_type: str) -> str:
    result = data.get("Result", {})
    lines = [f"结果数: {result.get('ResultCount', 0)}  耗时: {result.get('TimeCost', 0)}ms", ""]

    if search_type == "web":
        for item in result.get("WebResults") or []:
            lines.append(f"[{item.get('SortId', '')}] {item.get('Title', '')}")

            meta_parts = [part for part in [item.get("SiteName", ""), item.get("AuthInfoDes", "")] if part]
            if meta_parts:
                lines.append(f"    {' | '.join(meta_parts)}")

            if item.get("Url"):
                lines.append(f"    {item['Url']}")

            summary = item.get("Summary") or item.get("Snippet", "")
            if summary:
                lines.append(f"    {summary[:SUMMARY_PREVIEW_LIMIT]}")
            lines.append("")

    elif search_type == "image":
        for item in result.get("ImageResults") or []:
            image = item.get("Image", {})
            lines.append(f"[{item.get('SortId', '')}] {item.get('Title', '')}")
            if image.get("Url"):
                lines.append(f"    {image['Url']}")
            lines.append(f"    {image.get('Width', '?')}x{image.get('Height', '?')} ({image.get('Shape', '')})")
            lines.append("")

    return "\n".join(lines)


# ---- CLI ----

def main():
    _load_legacy_env_files()
    # 尝试从 skill 根目录加载 .env（与 scripts/ 同级）
    _skill_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    _load_legacy_env_file(os.path.join(_skill_root, ".env"))

    parser = argparse.ArgumentParser(
        description="火山引擎联网搜索 API\nhttps://www.volcengine.com/docs/85508/1650263\n"
        "凭证：Claw 中直接在聊天框发 Key 即可；或 WEB_SEARCH_API_KEY / --api-key"
    )
    parser.add_argument("query", help="搜索关键词")
    parser.add_argument("--type", "-t", default="web", choices=["web", "image"])
    parser.add_argument("--count", "-c", type=int, default=5)
    parser.add_argument(
        "--time-range",
        help="OneDay/OneWeek/OneMonth/OneYear/YYYY-MM-DD..YYYY-MM-DD",
    )
    parser.add_argument("--auth-level", type=int, default=0, choices=[0, 1])
    parser.add_argument("--query-rewrite", action="store_true", help="开启 Query 改写")
    parser.add_argument("--api-key", help="API Key（优先于环境变量 WEB_SEARCH_API_KEY）")
    parser.add_argument("--prompt-api-key", action="store_true", help="交互式输入 API Key（不回显）")

    args = parser.parse_args()

    if not args.query or not args.query.strip():
        print("Error: 请输入搜索词。", file=sys.stderr)
        sys.exit(1)
    if len(args.query) > 100:
        print("Error: 搜索词超过 100 字符，API 可能截断。建议精简后重试。", file=sys.stderr)
        sys.exit(1)

    if args.count < 1:
        print("Error: --count 需 ≥ 1。", file=sys.stderr)
        sys.exit(1)
    if args.type == "image" and args.count > 5:
        print("Error: image 类型最多返回 5 条，请调整 --count。", file=sys.stderr)
        sys.exit(1)
    if args.type == "web" and args.count > 50:
        print("Error: web 类型最多返回 50 条，请调整 --count。", file=sys.stderr)
        sys.exit(1)

    try:
        time_range = _validate_time_range(args.time_range)
    except ValueError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)

    api_key = _get_api_key(args.api_key)
    if not api_key and args.prompt_api_key:
        entered = getpass.getpass("API Key: ").strip()
        api_key = entered or None

    ak = sk = session_token = None
    if not api_key:
        ak, sk, session_token = _get_credentials()
        if not ak or not sk:
            print(
                "Error: 未找到凭证。请配置以下任一方式：\n"
                "1) 【推荐】若在 Claw 中使用：拿 Key 后直接在聊天框发给我即可，无需编辑配置\n"
                "2) API Key：设置 WEB_SEARCH_API_KEY 或传入 --api-key\n"
                "3) AK/SK：设置 VOLCENGINE_ACCESS_KEY 和 VOLCENGINE_SECRET_KEY\n"
                "开通指南：references/setup-guide.md 或 SKILL.md",
                file=sys.stderr,
            )
            sys.exit(1)

    body = build_body(
        query=args.query,
        search_type=args.type,
        count=args.count,
        time_range=time_range,
        auth_level=args.auth_level,
        query_rewrite=args.query_rewrite,
    )

    requests = _require_requests()
    try:
        data = do_search(body, api_key=api_key, ak=ak, sk=sk, session_token=session_token or "")
    except requests.exceptions.HTTPError as exc:
        print(f"HTTP Error: {exc}", file=sys.stderr)
        if exc.response is not None:
            status = exc.response.status_code
            body = exc.response.text or ""
            if status == 429:
                print(
                    "提示：请求频率过高触发限流，建议降频后重试。"
                    "详见 references/setup-guide.md",
                    file=sys.stderr,
                )
            elif status == 401 and ("InvalidAccessKey" in body or "invalid" in body.lower()):
                print(
                    "提示：AK/SK 无效或已失效。请检查 VOLCENGINE_ACCESS_KEY / VOLCENGINE_SECRET_KEY，"
                    "或改用 API Key（Claw 中可直接在聊天框发给我）。详见 references/setup-guide.md",
                    file=sys.stderr,
                )
            else:
                print(body, file=sys.stderr)
        sys.exit(1)
    except Exception as exc:
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)

    if data is None:
        print("No response.", file=sys.stderr)
        sys.exit(1)

    error = (data.get("ResponseMetadata") or {}).get("Error")
    if error:
        code = error.get("Code", "")
        msg = error.get("Message", "")
        print(f"API Error [{code}]: {msg}", file=sys.stderr)
        if str(code).lower() == "invalid_api_key" or "10403" in str(code):
            print(
                "提示：请确认 API Key 来自联网搜索控制台 https://console.volcengine.com/search-infinity/api-key ，"
                "而非火山方舟(Ark)。若在 Claw 中，可重新在聊天框发正确的 Key 给我。详见 references/setup-guide.md",
                file=sys.stderr,
            )
        elif "429" in str(code) or "flowlimit" in str(code).lower() or "100018" in str(code):
            print(
                "提示：请求频率过高触发限流，建议降频后重试。",
                file=sys.stderr,
            )
        else:
            hint = ERROR_HINTS.get(str(code))
            if hint:
                print(hint, file=sys.stderr)
        sys.exit(1)

    print(format_output(data, args.type))


if __name__ == "__main__":
    main()
