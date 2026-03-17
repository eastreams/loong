import { apiDelete, apiGet, apiPost } from "../../../lib/api/client";
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

interface SubmitTurnResponse {
  sessionId: string;
  message: ChatMessage;
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

  async submitTurn(sessionId: string, input: string): Promise<ChatMessage> {
    const response = await apiPost<
      ApiEnvelope<SubmitTurnResponse>,
      { input: string }
    >(`/api/chat/sessions/${encodeURIComponent(sessionId)}/turn`, { input });
    return response.data.message;
  },

  async deleteSession(sessionId: string): Promise<void> {
    await apiDelete(`/api/chat/sessions/${encodeURIComponent(sessionId)}`);
  },
};
