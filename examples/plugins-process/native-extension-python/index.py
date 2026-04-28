#!/usr/bin/env python3
import json
import sys


def build_extension_payload(operation, payload):
    if operation == "extension/event":
        return {
            "ok": True,
            "handled_event": payload.get("event", "unknown"),
        }
    if operation == "extension/command":
        command_name = payload.get("command_name", "extension")
        return {
            "text": f"{command_name} command stub"
        }
    if operation == "extension/resource":
        return {
            "commands": [],
            "tools": []
        }
    return {
        "error": f"unsupported method: {operation}"
    }


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    request = json.loads(line)
    method = request.get("method", "")
    payload = request.get("payload") or {}
    if method == "tools/call":
        operation = payload.get("operation", "")
        extension_payload = payload.get("payload") or {}
        response_payload = build_extension_payload(operation, extension_payload)
    else:
        response_payload = {"error": f"unsupported transport method: {method}"}
    response = {"method": method, "id": request.get("id"), "payload": response_payload}
    print(json.dumps(response), flush=True)
