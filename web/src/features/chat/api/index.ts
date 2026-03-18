import { apiDelete, apiGet, apiOpenStream, apiPost } from "../../../lib/api/client";
import type { ApiEnvelope } from "../../../lib/api/types";

export interface ChatSessionSummary {
  id: string;
  title: string;
  updatedAt: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | string;
  content: string;
  createdAt: string;
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
      outcome: "ok" | "error" | string;
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

interface StreamHandlers {
  onEvent: (event: ChatTurnStreamEvent) => void;
}

export const chatApi = {
  async listSessions(): Promise<ChatSessionSummary[]> {
    const response = await apiGet<ApiEnvelope<ChatSessionsResponse>>(
      "/api/chat/sessions",
    );
    return response.data.items;
  },

  async loadHistory(sessionId: string): Promise<ChatMessage[]> {
    const response = await apiGet<ApiEnvelope<ChatHistoryResponse>>(
      `/api/chat/sessions/${encodeURIComponent(sessionId)}/history`,
    );
    return response.data.messages;
  },

  async createSession(title?: string): Promise<string> {
    const response = await apiPost<
      ApiEnvelope<CreateChatSessionResponse>,
      { title?: string }
    >("/api/chat/sessions", title ? { title } : {});
    return response.data.sessionId;
  },

  async createTurn(sessionId: string, input: string): Promise<ChatTurnAccepted> {
    const response = await apiPost<
      ApiEnvelope<CreateTurnResponse>,
      { input: string }
    >(`/api/chat/sessions/${encodeURIComponent(sessionId)}/turn`, { input });
    return response.data;
  },

  async streamTurn(
    sessionId: string,
    turnId: string,
    handlers: StreamHandlers,
  ): Promise<void> {
    const response = await apiOpenStream(
      `/api/chat/sessions/${encodeURIComponent(sessionId)}/turns/${encodeURIComponent(turnId)}/stream`,
    );

    const reader = response.body?.getReader();
    if (!reader) {
      throw new Error("Streaming response body is not available");
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
        handlers.onEvent(JSON.parse(trimmed) as ChatTurnStreamEvent);
      }
    }

    const trailing = buffer.trim();
    if (trailing) {
      handlers.onEvent(JSON.parse(trailing) as ChatTurnStreamEvent);
    }
  },

  async deleteSession(sessionId: string): Promise<void> {
    await apiDelete(`/api/chat/sessions/${encodeURIComponent(sessionId)}`);
  },
};
