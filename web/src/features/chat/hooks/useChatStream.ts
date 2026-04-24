import { useState, useCallback, useRef } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../../lib/api/client";
import { resolveTokenHintEnv, resolveTokenHintPath } from "../../../lib/auth/tokenHint";
import {
  chatApi,
  isInternalAssistantRecordContent,
  looksLikeInternalAssistantRecordContent,
  stripInternalAssistantRecordPrefix,
  type ChatMessage,
  type ChatSessionSummary,
  type ChatTurnStreamEvent,
} from "../api";
import type { SessionViewState } from "./useChatSessions";

function upsertRecentTool(
  current: SessionViewState,
  tool: {
    toolId: string;
    label: string;
    status: "ok" | "error" | "pending";
    detail?: string;
  },
): SessionViewState["recentTools"] {
  const nextItem = {
    toolId: tool.toolId,
    label: tool.label,
    status: tool.status,
    finishedAt: new Date().toISOString(),
    detail: tool.detail,
  };

  return [
    nextItem,
    ...current.recentTools.filter((item) => item.toolId !== tool.toolId),
  ].slice(0, 6);
}

function resolveToolCompletionStatus(
  event: Extract<ChatTurnStreamEvent, { type: "tool.finished" }>,
): "ok" | "error" | "pending" {
  if (event.state === "needs_approval" || event.outcome === "needs_approval") {
    return "pending";
  }

  if (event.outcome === "ok" || event.state === "completed") {
    return "ok";
  }

  return "error";
}

function extractErrorHost(message: string): string | null {
  const match = message.match(/https?:\/\/([^/\s)]+)/i);
  return match?.[1] ?? null;
}

function resolveStreamPhaseFromLifecycleEvent(
  phase: string,
  currentPhase: SessionViewState["streamPhase"],
): SessionViewState["streamPhase"] {
  switch (phase) {
    case "completed":
    case "failed":
      return "idle";
    case "preparing":
    case "context_ready":
    case "requesting_provider":
    case "running_tools":
    case "requesting_followup_provider":
    case "finalizing_reply":
      return currentPhase === "streaming" ? "streaming" : "thinking";
    default:
      return currentPhase;
  }
}

function isTerminalStreamEvent(event: ChatTurnStreamEvent): boolean {
  return event.type === "turn.completed" || event.type === "turn.failed";
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
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
          tokenPath: resolveTokenHintPath(tokenPath),
          tokenEnv: resolveTokenHintEnv(tokenEnv),
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
  const internalContentBuffersRef = useRef<Map<string, string>>(new Map());

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
        case "turn.phase":
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            streamPhase: resolveStreamPhaseFromLifecycleEvent(
              event.phase,
              current.streamPhase,
            ),
          }));
          break;
        case "message.delta":
          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            streamPhase: "streaming",
            messages: current.messages.map((message) =>
              message.id === placeholderId
                ? (() => {
                    const bufferedInternalContent =
                      internalContentBuffersRef.current.get(placeholderId);
                    const nextContent = bufferedInternalContent
                      ? `${bufferedInternalContent}${event.delta}`
                      : `${message.content}${event.delta}`;
                    if (
                      ((message.content.trim().length === 0 ||
                        bufferedInternalContent) &&
                        looksLikeInternalAssistantRecordContent(event.delta)) ||
                      bufferedInternalContent ||
                      isInternalAssistantRecordContent(nextContent)
                    ) {
                      const visibleContent =
                        stripInternalAssistantRecordPrefix(nextContent);
                      if (visibleContent) {
                        internalContentBuffersRef.current.delete(placeholderId);
                        return { ...message, content: visibleContent };
                      }
                      internalContentBuffersRef.current.set(placeholderId, nextContent);
                      return {
                        ...message,
                        content: "",
                      };
                    }
                    return { ...message, content: nextContent };
                  })()
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
          updateSessionViewState(targetSessionId, (current) => {
            const status = resolveToolCompletionStatus(event);
            return {
              ...current,
              activeTools: current.activeTools.map((item) =>
                item.toolId === event.toolId
                  ? {
                      ...item,
                      label: event.label,
                      status,
                    }
                  : item,
              ),
              recentTools: upsertRecentTool(current, {
                toolId: event.toolId,
                label: event.label,
                status,
                detail:
                  event.detail ??
                  (status === "ok"
                    ? t("chat.recentTools.detail.ok")
                    : status === "pending"
                      ? t("chat.recentTools.detail.pending", {
                          defaultValue: "Waiting for approval",
                        })
                      : t("chat.recentTools.detail.error")),
              }),
            };
          });
          break;
        case "turn.completed":
          internalContentBuffersRef.current.delete(placeholderId);
          if (
            event.message.role === "assistant" &&
            isInternalAssistantRecordContent(event.message.content)
          ) {
            const visibleContent = stripInternalAssistantRecordPrefix(
              event.message.content,
            );
            if (visibleContent) {
              updateSessionViewState(targetSessionId, (current) => ({
                messages: current.messages.map((message) =>
                  message.id === placeholderId
                    ? { ...event.message, content: visibleContent }
                    : message,
                ),
                activeTools: [],
                recentTools: current.recentTools,
                pendingAssistantId: null,
                streamPhase: "idle",
              }));
              break;
            }
            updateSessionViewState(targetSessionId, (current) => ({
              ...current,
              messages: current.messages.filter((message) => message.id !== placeholderId),
              activeTools: [],
              pendingAssistantId: null,
              streamPhase: "idle",
            }));
            break;
          }
          updateSessionViewState(targetSessionId, (current) => ({
            messages: current.messages.map((message) =>
              message.id === placeholderId ? event.message : message,
            ),
            activeTools: [],
            recentTools: current.recentTools,
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
          break;
        case "turn.failed":
          internalContentBuffersRef.current.delete(placeholderId);
          updateSessionViewState(targetSessionId, (current) => ({
            messages: current.messages.filter((message) => message.id !== placeholderId),
            activeTools: [],
            recentTools: current.activeTools.reduce(
              (acc, item) => [
                {
                  toolId: item.toolId,
                  label: item.label,
                  status: "error" as const,
                  finishedAt: new Date().toISOString(),
                  detail: t("chat.recentTools.detail.interrupted"),
                },
                ...acc.filter((recent) => recent.toolId !== item.toolId),
              ].slice(0, 6),
              current.recentTools,
            ),
            pendingAssistantId: null,
            streamPhase: "idle",
          }));
          setError(event.message);
          break;
      }
    },
    [setError, t, updateSessionViewState],
  );

  const reconcileUnexpectedStreamClose = useCallback(
    async (targetSessionId: string, placeholderId: string) => {
      const settleDelaysMs = [0, 150, 450, 900];

      for (const settleDelayMs of settleDelaysMs) {
        if (settleDelayMs > 0) {
          await delay(settleDelayMs);
        }

        try {
          const latestMessages = await chatApi.loadHistory(targetSessionId);
          const hasAssistantReply = latestMessages.some(
            (message) => message.role === "assistant" && message.content.trim().length > 0,
          );

          updateSessionViewState(targetSessionId, (current) => ({
            ...current,
            messages: latestMessages,
            activeTools: [],
            pendingAssistantId: null,
            streamPhase: "idle",
          }));

          if (hasAssistantReply) {
            return true;
          }
        } catch {
          // Keep retrying briefly in case the final turn is still settling into sqlite.
        }
      }

      updateSessionViewState(targetSessionId, (current) => ({
        ...current,
        messages: current.messages.filter((message) => message.id !== placeholderId),
        activeTools: [],
        pendingAssistantId: null,
        streamPhase: "idle",
      }));
      setError(t("chat.errors.streamEndedUnexpectedly"));
      return false;
    },
    [setError, t, updateSessionViewState],
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
            recentTools: [],
            pendingAssistantId: placeholderAssistantId,
            streamPhase: "connecting",
          }));
        }

        const acceptedTurn = await chatApi.createTurn(targetSessionId, input);
        turnAccepted = true;
        let receivedTerminalEvent = false;

        abortControllerRef.current = new AbortController();

        await chatApi.streamTurn(
          targetSessionId,
          acceptedTurn.turnId,
          {
            onEvent: (event) => {
              if (isTerminalStreamEvent(event)) {
                receivedTerminalEvent = true;
              }
              handleStreamEvent(targetSessionId!, event, placeholderAssistantId);
            },
          },
          {
            signal: abortControllerRef.current.signal,
          },
        );

        if (!receivedTerminalEvent) {
          await reconcileUnexpectedStreamClose(targetSessionId, placeholderAssistantId);
        }

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
      reconcileUnexpectedStreamClose,
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
