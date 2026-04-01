import { useState, useCallback, useRef } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../../lib/api/client";
import {
  chatApi,
  type ChatMessage,
  type ChatSessionSummary,
  type ChatTurnStreamEvent,
} from "../api";
import type { SessionViewState } from "./useChatSessions";

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
      : t("auth.invalidBody", {
          tokenPath: tokenPath ?? "",
          tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN",
        });
  }

  const rawMessage = error instanceof Error ? error.message : "Failed to send message";
  if (rawMessage.includes("transport_failure")) {
    const host = extractErrorHost(rawMessage);
    return t("chat.errors.transportFailure", {
      host: host ?? t("chat.errors.providerHostFallback"),
    });
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
  updateSessionViewState: (
    sessionId: string,
    updater: (current: SessionViewState) => SessionViewState,
  ) => void;
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

  const handleStreamEvent = useCallback(
    (targetSessionId: string, event: ChatTurnStreamEvent, placeholderId: string) => {
      switch (event.type) {
        case "turn.started":
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            streamPhase: "thinking",
          }));
          break;
        case "message.delta":
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            streamPhase: "streaming",
            messages: current.messages.map((message) =>
              message.id === placeholderId
                ? { ...message, content: `${message.content}${event.delta}` }
                : message,
            ),
          }));
          break;
        case "tool.started":
          updateSessionViewState(targetSessionId, (current) => {
            const existing = current.activeTools.find(
              (item) => item.toolId === event.toolId,
            );
            return {
              ...current,
              streamPhase:
                current.streamPhase === "connecting"
                  ? "thinking"
                  : current.streamPhase,
              activeTools: existing
                ? current.activeTools.map((item) =>
                    item.toolId === event.toolId
                      ? { ...item, label: event.label, status: "running" }
                      : item,
                  )
                : [
                    ...current.activeTools,
                    {
                      toolId: event.toolId,
                      label: event.label,
                      status: "running" as const,
                    },
                  ],
            };
          });
          break;
        case "tool.finished":
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            activeTools: current.activeTools.map((item) =>
              item.toolId === event.toolId
                ? {
                    ...item,
                    label: event.label,
                    status: event.outcome === "ok" ? ("ok" as const) : ("error" as const),
                  }
                : item,
            ),
          }));
          break;
        case "turn.completed":
          updateSessionViewState(targetSessionId, (current) => ({
            messages: current.messages.map((message) =>
              message.id === placeholderId ? event.message : message,
            ),
            activeTools: [],
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
          break;
        case "turn.failed":
          updateSessionViewState(targetSessionId, (current) => ({
            messages: current.messages.filter((message) => message.id !== placeholderId),
            activeTools: [],
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
          setError(event.message);
          break;
      }
    },
    [setError, updateSessionViewState],
  );

  const sendMessage = useCallback(
    async (input: string) => {
      if (!input.trim() || isSubmitting || !canAccessProtectedApi) return;

      const nowIso = new Date().toISOString();
      const optimisticUserMessage: ChatMessage = {
        id: `local-user-${Date.now()}`,
        role: "user",
        content: input,
        createdAt: nowIso,
      };
      const placeholderAssistantId = `local-assistant-${Date.now()}`;
      const placeholderAssistantMessage: ChatMessage = {
        id: placeholderAssistantId,
        role: "assistant",
        content: "",
        createdAt: nowIso,
      };

      setError(null);
      setIsSubmitting(true);

      let targetSessionId = sessionId;
      let turnAccepted = false;
      let createdSessionId: string | null = null;
      const initialMessagesForNewSession = [
        optimisticUserMessage,
        placeholderAssistantMessage,
      ];

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

        const acceptedTurn = await chatApi.createTurn(targetSessionId, input);
        turnAccepted = true;

        abortControllerRef.current = new AbortController();

        await chatApi.streamTurn(
          targetSessionId,
          acceptedTurn.turnId,
          {
            onEvent: (event) =>
              handleStreamEvent(targetSessionId!, event, placeholderAssistantId),
          },
          {
            signal: abortControllerRef.current.signal,
          },
        );

        updateSessionViewState(targetSessionId, (current) => ({
          ...current,
          activeTools: [],
        }));
        await refreshSessions(targetSessionId);
        return true;
      } catch (err) {
        if (err instanceof Error && err.name === "AbortError") {
          if (targetSessionId) {
            updateSessionViewState(targetSessionId, (current) => ({
              ...current,
              messages: current.messages.filter(
                (message) => message.id !== placeholderAssistantId,
              ),
              activeTools: [],
              pendingAssistantId: null,
              streamPhase: "idle",
            }));
          }
          return turnAccepted;
        }

        const friendlyError = toFriendlyChatError(
          err,
          t,
          markUnauthorized,
          authMode,
          tokenPath,
          tokenEnv,
        );
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
              messages: current.messages.filter(
                (message) => message.id !== placeholderAssistantId,
              ),
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
              (message) =>
                message.id !== optimisticUserMessage.id &&
                message.id !== placeholderAssistantId,
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
      } finally {
        setIsSubmitting(false);
        abortControllerRef.current = null;
      }
    },
    [
      authMode,
      canAccessProtectedApi,
      handleStreamEvent,
      isSubmitting,
      markUnauthorized,
      refreshSessions,
      removeSession,
      selectSession,
      sessionId,
      setError,
      t,
      tokenEnv,
      tokenPath,
      updateSessionViewState,
      upsertSession,
    ],
  );

  return { isSubmitting, sendMessage, stopStream };
}
