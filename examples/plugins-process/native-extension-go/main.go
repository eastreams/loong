package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

type requestFrame struct {
	Method  string         `json:"method"`
	ID      any            `json:"id"`
	Payload map[string]any `json:"payload"`
}

type responseFrame struct {
	Method  string `json:"method"`
	ID      any    `json:"id"`
	Payload any    `json:"payload"`
}

func buildExtensionPayload(operation string, payload map[string]any) any {
	switch operation {
	case "extension/event":
		event, _ := payload["event"].(string)
		if event == "" {
			event = "unknown"
		}
		return map[string]any{"ok": true, "handled_event": event}
	case "extension/command":
		commandName, _ := payload["command_name"].(string)
		if commandName == "" {
			commandName = "extension"
		}
		return map[string]any{"text": fmt.Sprintf("%s command stub", commandName)}
	case "extension/resource":
		return map[string]any{"commands": []any{}, "tools": []any{}}
	default:
		return map[string]any{"error": fmt.Sprintf("unsupported method: %s", operation)}
	}
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			continue
		}
		var request requestFrame
		if err := json.Unmarshal([]byte(line), &request); err != nil {
			continue
		}
		payload := request.Payload
		if payload == nil {
			payload = map[string]any{}
		}
		var responsePayload any
		if request.Method == "tools/call" {
			operation, _ := payload["operation"].(string)
			extensionPayload, _ := payload["payload"].(map[string]any)
			if extensionPayload == nil {
				extensionPayload = map[string]any{}
			}
			responsePayload = buildExtensionPayload(operation, extensionPayload)
		} else {
			responsePayload = map[string]any{"error": fmt.Sprintf("unsupported transport method: %s", request.Method)}
		}
		response := responseFrame{Method: request.Method, ID: request.ID, Payload: responsePayload}
		encoded, err := json.Marshal(response)
		if err != nil {
			continue
		}
		fmt.Println(string(encoded))
	}
}
