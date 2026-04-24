import {
  apiDelete,
  apiGetData,
  apiOpenStream,
  apiPostData,
  buildApiUrl,
  type ApiRequestOptions,
} from "../../../lib/api/client";
import { ChatMessage } from "../types";
export type { ChatMessage };

const CHAT_READ_TIMEOUT_MS = 15_000;
const CHAT_WRITE_TIMEOUT_MS = 30_000;

export interface ChatSessionSummary {
  id: string;
  title: string;
  updatedAt: string;
}

export interface ChatTurnAccepted {
  sessionId: string;
  turnId: string;
  status: "accepted" | string;
}

export type ChatTurnStreamEvent =
  | {
      type: "turn.started";
      turnId: string;
      sessionId: string;
      createdAt: string;
    }
  | {
      type: "turn.phase";
      turnId: string;
      phase: string;
      providerRound?: number;
      lane?: string;
      toolCallCount?: number;
      messageCount?: number;
      estimatedTokens?: number;
    }
  | {
      type: "message.delta";
      turnId: string;
      role: "assistant" | string;
      delta: string;
    }
  | {
      type: "tool.started";
      turnId: string;
      toolId: string;
      label: string;
    }
  | {
      type: "tool.finished";
      turnId: string;
      toolId: string;
      label: string;
      outcome: "ok" | "error" | "needs_approval" | "denied" | string;
      state?: "completed" | "needs_approval" | "denied" | "failed" | "interrupted" | string;
      detail?: string;
    }
  | {
      type: "turn.completed";
      turnId: string;
      message: ChatMessage;
    }
  | {
      type: "turn.failed";
      turnId: string;
      code: string;
      message: string;
    };

interface ChatSessionsResponse {
  items: ChatSessionSummary[];
}

interface ChatHistoryResponse {
  sessionId: string;
  messages: ChatMessage[];
}

interface CreateChatSessionResponse {
  sessionId: string;
}

type CreateTurnResponse = ChatTurnAccepted;

interface CreateTurnRequest {
  input: string;
}

interface StreamHandlers {
  onEvent: (event: ChatTurnStreamEvent) => void;
}

export function isInternalAssistantRecordContent(content: string): boolean {
  const trimmed = content.trim();
  if (!trimmed) {
    return false;
  }
  if (trimmed.startsWith("[tool_result]")) {
    return true;
  }
  if (isInternalAssistantControlText(trimmed)) {
    return true;
  }

  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    const markedInternal = parsed._loong_internal === true;
    const internalType =
      parsed.type === "conversation_event" ||
      parsed.type === "tool_decision" ||
      parsed.type === "tool_outcome";
    return (
      markedInternal ||
      internalType
    );
  } catch {
    return false;
  }
}

function isInternalAssistantControlText(trimmedContent: string): boolean {
  return [
    "[tool_request]",
    "[tool_failure]",
    "[tool_recovery]",
    "[tool_loop_guard]",
    "[tool_loop_warning]",
    "[tool_discovery_runtime]",
  ].some((prefix) => trimmedContent.startsWith(prefix));
}

export function stripInternalAssistantRecordPrefix(content: string): string {
  let remaining = content.trimStart();
  if (!remaining.startsWith("[tool_result]")) {
    return content;
  }

  while (remaining.startsWith("[tool_result]")) {
    const nextSeparator = remaining.indexOf("\n\n");
    if (nextSeparator < 0) {
      return "";
    }
    remaining = remaining.slice(nextSeparator).trimStart();
  }

  return remaining;
}

export function looksLikeInternalAssistantRecordContent(content: string): boolean {
  const trimmed = content.trimStart();
  return (
    trimmed.startsWith('{"_loong_internal"') ||
    trimmed.startsWith('{"type":"conversation_event"') ||
    trimmed.startsWith('{"type":"tool_decision"') ||
    trimmed.startsWith('{"type":"tool_outcome"') ||
    trimmed.startsWith("[tool_result]") ||
    isInternalAssistantControlText(trimmed)
  );
}

function withDefaultTimeout(
  request: ApiRequestOptions | undefined,
  timeoutMs: number,
): ApiRequestOptions {
  return {
    ...request,
    timeoutMs: request?.timeoutMs ?? timeoutMs,
  };
}

function parseStreamEvent(rawLine: string, requestTarget: string): ChatTurnStreamEvent {
  try {
    return JSON.parse(rawLine) as ChatTurnStreamEvent;
  } catch {
    throw new Error(`Failed to parse stream event (${requestTarget}).`);
  }
}

export const chatApi = {
  async listSessions(request?: ApiRequestOptions): Promise<ChatSessionSummary[]> {
    const response = await apiGetData<ChatSessionsResponse>(
      "/api/chat/sessions",
      withDefaultTimeout(request, CHAT_READ_TIMEOUT_MS),
    );
    return response.items;
  },

  async loadHistory(sessionId: string, request?: ApiRequestOptions): Promise<ChatMessage[]> {
    const response = await apiGetData<ChatHistoryResponse>(
      `/api/chat/sessions/${encodeURIComponent(sessionId)}/history`,
      withDefaultTimeout(request, CHAT_READ_TIMEOUT_MS),
    );
    return response.messages
      .map((message) =>
        message.role === "assistant"
          ? {
              ...message,
              content: stripInternalAssistantRecordPrefix(message.content),
            }
          : message,
      )
      .filter(
        (message) =>
          !(
            message.role === "assistant" &&
            isInternalAssistantRecordContent(message.content)
          ),
      );
  },

  async createSession(title?: string, request?: ApiRequestOptions): Promise<string> {
    const response = await apiPostData<CreateChatSessionResponse, { title?: string }>(
      "/api/chat/sessions",
      title ? { title } : {},
      withDefaultTimeout(request, CHAT_WRITE_TIMEOUT_MS),
    );
    return response.sessionId;
  },

  async createTurn(
    sessionId: string,
    input: string,
    request?: ApiRequestOptions,
  ): Promise<ChatTurnAccepted> {
    return apiPostData<CreateTurnResponse, CreateTurnRequest>(
      `/api/chat/sessions/${encodeURIComponent(sessionId)}/turn`,
      { input },
      withDefaultTimeout(request, CHAT_WRITE_TIMEOUT_MS),
    );
  },

  async streamTurn(
    sessionId: string,
    turnId: string,
    handlers: StreamHandlers,
    request?: ApiRequestOptions,
  ): Promise<void> {
    const streamPath = `/api/chat/sessions/${encodeURIComponent(sessionId)}/turns/${encodeURIComponent(turnId)}/stream`;
    const requestTarget = `GET ${buildApiUrl(streamPath)}`;
    const response = await apiOpenStream(streamPath, request);

    const reader = response.body?.getReader();
    if (!reader) {
      throw new Error(`Streaming response body is not available (${requestTarget}).`);
    }

    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) {
          continue;
        }
        handlers.onEvent(parseStreamEvent(trimmed, requestTarget));
      }
    }

    const trailing = buffer.trim();
    if (trailing) {
      handlers.onEvent(parseStreamEvent(trailing, requestTarget));
    }
  },

  async deleteSession(sessionId: string, request?: ApiRequestOptions): Promise<void> {
    await apiDelete(
      `/api/chat/sessions/${encodeURIComponent(sessionId)}`,
      withDefaultTimeout(request, CHAT_WRITE_TIMEOUT_MS),
    );
  },

};
