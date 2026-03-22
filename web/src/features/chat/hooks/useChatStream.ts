import { useState, useCallback, useRef } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../../lib/api/client";
import {
  chatApi,
  type ChatMessage,
  type ChatSessionSummary,
  type ChatTurnStreamEvent,
} from "../api";
import type { SessionViewState, ActiveToolStatus } from "./useChatSessions";

type ToolAssistIntent = "filesystem" | "repo_search" | "shell" | "web";

function detectToolAssistIntent(input: string): ToolAssistIntent | null {
  const normalized = input.trim().toLowerCase();
  if (!normalized) return null;

  if (/(https?:\/\/|网页|网站|链接|打开网页|打开网站|browse|browser|web fetch|webfetch|url|summarize this page)/i.test(normalized)) return "web";
  if (/(ls|pwd|终端|命令行|shell|执行命令|run command|current directory|列出当前目录)/i.test(normalized)) return "shell";
  if (/(搜索|查找|grep|rg|ripgrep|仓库|代码里|repository|repo|source code|find in code)/i.test(normalized)) return "repo_search";
  if (/(文件|目录|读取|查看|打开文件|readme|cargo\.toml|package\.json|file|directory|folder|read the file)/i.test(normalized)) return "filesystem";

  return null;
}

function buildToolAssistHint(intent: ToolAssistIntent | null): string | null {
  if (!intent) return null;
  const generic = "If tools are needed, first use tool.search to discover the right runtime tool, then use tool.invoke instead of claiming the tool is unavailable.";
  switch (intent) {
    case "filesystem": return `${generic} This request looks file- or directory-related, so prefer discovering a file or shell-oriented tool before answering from memory.`;
    case "repo_search": return `${generic} This request looks like repository/code search, so prefer discovering a search, file, or shell-oriented tool before answering from memory.`;
    case "shell": return `${generic} This request explicitly asks for a shell-style action, so prefer discovering a shell-oriented tool and executing it if policy allows.`;
    case "web": return `${generic} This request looks web-related, so prefer discovering a browser or web-fetch tool before answering from memory.`;
    default: return generic;
  }
}

function extractErrorHost(message: string): string | null {
  const match = message.match(/https?:\/\/([^/\s)]+)/i);
  return match?.[1] ?? null;
}

function toFriendlyChatError(
  error: unknown,
  t: TFunction,
  markUnauthorized: () => void,
  authMode: string | null,
  tokenPath: string | null,
  tokenEnv: string | null,
): string {
  if (error instanceof ApiRequestError && error.status === 401) {
    markUnauthorized();
    return authMode === "same_origin_session"
      ? t("auth.sessionInvalidBody")
      : t("auth.invalidBody", { tokenPath: tokenPath ?? "", tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN" });
  }

  const rawMessage = error instanceof Error ? error.message : "Failed to send message";
  if (rawMessage.includes("transport_failure")) {
    const host = extractErrorHost(rawMessage);
    return t("chat.errors.transportFailure", { host: host ?? t("chat.errors.providerHostFallback") });
  }
  return rawMessage;
}

interface UseChatStreamParams {
  t: TFunction;
  sessionId: string | null;
  canAccessProtectedApi: boolean;
  markUnauthorized: () => void;
  authMode: string | null;
  tokenPath: string | null;
  tokenEnv: string | null;
  toolAssistEnabled: boolean;
  updateSessionViewState: (sessionId: string, updater: (current: SessionViewState) => SessionViewState) => void;
  selectSession: (sessionId: string | null) => void;
  upsertSession: (session: ChatSessionSummary) => void;
  removeSession: (sessionId: string) => void;
  refreshSessions: (preferredSessionId?: string) => Promise<void>;
  setError: (error: string | null) => void;
}

export function useChatStream({
  t,
  sessionId,
  canAccessProtectedApi,
  markUnauthorized,
  authMode,
  tokenPath,
  tokenEnv,
  toolAssistEnabled,
  updateSessionViewState,
  selectSession,
  upsertSession,
  removeSession,
  refreshSessions,
  setError,
}: UseChatStreamParams) {
  const [isSubmitting, setIsSubmitting] = useState(false);
  const abortControllerRef = useRef<AbortController | null>(null);

  const stopStream = useCallback(() => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      abortControllerRef.current = null;
    }
  }, []);

  const handleStreamEvent = useCallback((
    targetSessionId: string,
    event: ChatTurnStreamEvent,
    placeholderId: string,
  ) => {
    switch (event.type) {
      case "turn.started":
        updateSessionViewState(targetSessionId, (current) => ({ ...current, streamPhase: "thinking" }));
        break;
      case "message.delta":
        updateSessionViewState(targetSessionId, (current) => ({
          ...current,
          streamPhase: "streaming",
          messages: current.messages.map((m) => m.id === placeholderId ? { ...m, content: `${m.content}${event.delta}` } : m),
        }));
        break;
      case "tool.started":
        updateSessionViewState(targetSessionId, (current) => {
          const existing = current.activeTools.find((item) => item.toolId === event.toolId);
          return {
            ...current,
            streamPhase: current.streamPhase === "connecting" ? "thinking" : current.streamPhase,
            activeTools: existing
              ? current.activeTools.map((item) => item.toolId === event.toolId ? { ...item, label: event.label, status: "running" } : item)
              : [...current.activeTools, { toolId: event.toolId, label: event.label, status: "running" as const }],
          };
        });
        break;
      case "tool.finished":
        updateSessionViewState(targetSessionId, (current) => ({
          ...current,
          activeTools: current.activeTools.map((item) =>
            item.toolId === event.toolId ? { ...item, label: event.label, status: event.outcome === "ok" ? "ok" as const : "error" as const } : item
          ),
        }));
        break;
      case "turn.completed":
        updateSessionViewState(targetSessionId, (current) => ({
          messages: current.messages.map((m) => m.id === placeholderId ? event.message : m),
          activeTools: [],
          pendingAssistantId: null,
          streamPhase: "idle",
        }));
        break;
      case "turn.failed":
        updateSessionViewState(targetSessionId, (current) => ({
          messages: current.messages.filter((m) => m.id !== placeholderId),
          activeTools: [],
          pendingAssistantId: null,
          streamPhase: "idle",
        }));
        setError(event.message);
        break;
    }
  }, [updateSessionViewState, setError]);

  const sendMessage = useCallback(async (input: string) => {
    if (!input.trim() || isSubmitting || !canAccessProtectedApi) return;

    const nowIso = new Date().toISOString();
    const optimisticUserMessage: ChatMessage = { id: `local-user-${Date.now()}`, role: "user", content: input, createdAt: nowIso };
    const placeholderAssistantId = `local-assistant-${Date.now()}`;
    const placeholderAssistantMessage: ChatMessage = { id: placeholderAssistantId, role: "assistant", content: "", createdAt: nowIso };

    setError(null);
    setIsSubmitting(true);

    let targetSessionId = sessionId;
    let turnAccepted = false;
    let createdSessionId: string | null = null;
    const initialMessagesForNewSession = [optimisticUserMessage, placeholderAssistantMessage];

    try {
      if (targetSessionId) {
        updateSessionViewState(targetSessionId, (current) => ({
          ...current,
          messages: [...current.messages, ...initialMessagesForNewSession],
          activeTools: [],
          pendingAssistantId: placeholderAssistantId,
          streamPhase: "connecting",
        }));
      }

      const toolAssistHint = toolAssistEnabled ? buildToolAssistHint(detectToolAssistIntent(input)) : null;
      
      if (!targetSessionId) {
        const optimisticTitle = input.trim().slice(0, 48) || "New session";
        targetSessionId = await chatApi.createSession(optimisticTitle);
        createdSessionId = targetSessionId;
        upsertSession({
          id: targetSessionId,
          title: optimisticTitle,
          updatedAt: nowIso,
        });
        selectSession(targetSessionId);
        updateSessionViewState(targetSessionId, () => ({
          messages: initialMessagesForNewSession,
          activeTools: [],
          pendingAssistantId: placeholderAssistantId,
          streamPhase: "connecting",
        }));
      }

      const acceptedTurn = await chatApi.createTurn(targetSessionId, input, toolAssistHint ?? undefined);
      turnAccepted = true;
      
      abortControllerRef.current = new AbortController();
      
      await chatApi.streamTurn(targetSessionId, acceptedTurn.turnId, {
        onEvent: (event) => handleStreamEvent(targetSessionId!, event, placeholderAssistantId),
      }, {
        signal: abortControllerRef.current.signal,
      });

      updateSessionViewState(targetSessionId, (current) => ({ ...current, activeTools: [] }));
      await refreshSessions(targetSessionId);
      return true;
    } catch (err) {
      if (err instanceof Error && err.name === "AbortError") {
        if (targetSessionId) {
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            messages: current.messages.filter((m) => m.id !== placeholderAssistantId),
            activeTools: [],
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
        }
        return turnAccepted;
      } else {
        const friendlyError = toFriendlyChatError(err, t, markUnauthorized, authMode, tokenPath, tokenEnv);
        setError(friendlyError);
        if (turnAccepted && targetSessionId) {
          try {
            const latestMessages = await chatApi.loadHistory(targetSessionId);
            updateSessionViewState(targetSessionId, (current) => ({
              ...current,
              messages: latestMessages,
              activeTools: [],
              pendingAssistantId: null,
              streamPhase: "idle",
            }));
          } catch {
            updateSessionViewState(targetSessionId, (current) => ({
              ...current,
              messages: current.messages.filter((m) => m.id !== placeholderAssistantId),
              activeTools: [],
              pendingAssistantId: null,
              streamPhase: "idle",
            }));
          }
          await refreshSessions(targetSessionId);
          return true;
        }

        if (targetSessionId) {
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            messages: current.messages.filter(
              (m) => m.id !== optimisticUserMessage.id && m.id !== placeholderAssistantId,
            ),
            activeTools: [],
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
        }

        if (createdSessionId) {
          removeSession(createdSessionId);
        }
        return false;
      }
    } finally {
      setIsSubmitting(false);
      abortControllerRef.current = null;
    }
  }, [isSubmitting, canAccessProtectedApi, sessionId, toolAssistEnabled, updateSessionViewState, selectSession, upsertSession, removeSession, t, markUnauthorized, authMode, tokenPath, tokenEnv, setError, handleStreamEvent, refreshSessions]);

  return { isSubmitting, sendMessage, stopStream };
}
