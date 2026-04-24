import { useEffect, useRef, useState, useCallback } from "react";
import type { TFunction } from "i18next";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { ApiRequestError } from "../../../lib/api/client";
import { resolveTokenHintEnv, resolveTokenHintPath } from "../../../lib/auth/tokenHint";
import {
  chatApi,
  type ChatMessage,
  type ChatSessionSummary,
} from "../api";
import { CHAT_MASCOT_SESSION_ID } from "../mascotProfile";

export type StreamPhase = "idle" | "connecting" | "thinking" | "streaming";

export interface ActiveToolStatus {
  toolId: string;
  label: string;
  status: "running" | "ok" | "error" | "pending";
}

export interface RecentToolStatus {
  toolId: string;
  label: string;
  status: "ok" | "error" | "pending";
  finishedAt: string;
  detail?: string;
}

export interface SessionViewState {
  messages: ChatMessage[];
  activeTools: ActiveToolStatus[];
  recentTools: RecentToolStatus[];
  pendingAssistantId: string | null;
  streamPhase: StreamPhase;
}

const CHAT_SELECTED_SESSION_STORAGE_KEY = "loongclaw.web.chat.selectedSessionId";

function readStoredSelectedSessionId(): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  const value = window.sessionStorage.getItem(CHAT_SELECTED_SESSION_STORAGE_KEY);
  return value && value.trim() ? value : null;
}

function filterVisibleSessions(sessions: ChatSessionSummary[]): ChatSessionSummary[] {
  return sessions.filter((session) => session.id !== CHAT_MASCOT_SESSION_ID);
}

export function useChatSessions(t: TFunction) {
  const {
    canAccessProtectedApi,
    authRevision,
    markUnauthorized,
    status,
    authMode,
    tokenPath,
    tokenEnv,
  } = useWebConnection();

  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(() =>
    readStoredSelectedSessionId(),
  );
  const [isLoadingSessions, setIsLoadingSessions] = useState(true);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const [deletingSessionId, setDeletingSessionId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // View state for each session (including background ones)
  const sessionViewStateRef = useRef<Map<string, SessionViewState>>(new Map());

  // Currently visible states
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [activeTools, setActiveTools] = useState<ActiveToolStatus[]>([]);
  const [recentTools, setRecentTools] = useState<RecentToolStatus[]>([]);
  const [pendingAssistantId, setPendingAssistantId] = useState<string | null>(null);
  const [streamPhase, setStreamPhase] = useState<StreamPhase>("idle");

  const selectedSessionIdRef = useRef<string | null>(selectedSessionId);
  const previousSelectedSessionIdRef = useRef<string | null>(selectedSessionId);

  // Sync refs with state for use in callbacks
  const currentViewStateRef = useRef<SessionViewState>({
    messages,
    activeTools,
    recentTools,
    pendingAssistantId,
    streamPhase,
  });

  // Sync Ref synchronously instead of during commit to avoid stale reads during fast streaming
  const syncCurrentViewState = useCallback((state: SessionViewState) => {
    currentViewStateRef.current = state;
  }, []);

  const refreshSessions = useCallback(async (preferredSessionId?: string) => {
    try {
      const loadedSessions = filterVisibleSessions(await chatApi.listSessions());
      setSessions(loadedSessions);
      if (preferredSessionId) {
        setSelectedSessionId(preferredSessionId);
      } else if (!selectedSessionId && loadedSessions.length > 0) {
        setSelectedSessionId(loadedSessions[0].id);
      }
    } catch (err) {
      // Ignore background refresh errors
    }
  }, [selectedSessionId]);

  const upsertSession = useCallback((session: ChatSessionSummary) => {
    setSessions((current) => {
      const existingIndex = current.findIndex((item) => item.id === session.id);
      if (existingIndex === -1) {
        return [session, ...current];
      }

      const next = [...current];
      next.splice(existingIndex, 1);
      return [session, ...next];
    });
  }, []);

  const updateSessionViewState = useCallback((
    sessionId: string,
    updater: (current: SessionViewState) => SessionViewState,
  ) => {
    const isCurrent = selectedSessionIdRef.current === sessionId;
    const baseState = isCurrent
      ? currentViewStateRef.current
      : sessionViewStateRef.current.get(sessionId) ?? {
          messages: [],
          activeTools: [],
          recentTools: [],
          pendingAssistantId: null,
          streamPhase: "idle" as StreamPhase,
        };

    const nextState = updater(baseState);
    sessionViewStateRef.current.set(sessionId, nextState);

    if (isCurrent) {
      syncCurrentViewState(nextState);
      setMessages(nextState.messages);
      setActiveTools(nextState.activeTools);
      setRecentTools(nextState.recentTools);
      setPendingAssistantId(nextState.pendingAssistantId);
      setStreamPhase(nextState.streamPhase);
    }
  }, [syncCurrentViewState]);

  const selectSession = useCallback((sessionId: string | null) => {
    setSelectedSessionId(sessionId);
  }, []);

  // Initial load
  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      setSessions([]);
      setSelectedSessionId(null);
      setMessages([]);
      setActiveTools([]);
      setIsLoadingSessions(false);
      setError(
        status === "unauthorized"
          ? authMode === "same_origin_session"
            ? t("auth.sessionInvalidBody")
            : t("auth.invalidBody", {
                tokenPath: resolveTokenHintPath(tokenPath),
                tokenEnv: resolveTokenHintEnv(tokenEnv),
              })
          : t("auth.requiredBody"),
      );
      return () => {
        cancelled = true;
      };
    }

    async function loadSessions() {
      setIsLoadingSessions(true);
      setError(null);
      try {
        const loadedSessions = filterVisibleSessions(await chatApi.listSessions());
        if (cancelled) return;
        setSessions(loadedSessions);
        setSelectedSessionId((current) => {
          if (current && loadedSessions.some((s) => s.id === current)) return current;
          const stored = readStoredSelectedSessionId();
          if (stored && loadedSessions.some((s) => s.id === stored)) return stored;
          return loadedSessions[0]?.id ?? null;
        });
      } catch (loadError) {
        if (!cancelled) {
          if (loadError instanceof ApiRequestError && loadError.status === 401) {
            markUnauthorized();
            setError(
              authMode === "same_origin_session"
                ? t("auth.sessionInvalidBody")
                : t("auth.invalidBody", {
                    tokenPath: resolveTokenHintPath(tokenPath),
                    tokenEnv: resolveTokenHintEnv(tokenEnv),
                  }),
            );
          } else {
            setError(loadError instanceof Error ? loadError.message : "Failed to load sessions");
          }
        }
      } finally {
        if (!cancelled) setIsLoadingSessions(false);
      }
    }

    void loadSessions();
    return () => {
      cancelled = true;
    };
  }, [authRevision, canAccessProtectedApi, markUnauthorized, status, t, authMode, tokenEnv, tokenPath]);

  // Session switching logic
  useEffect(() => {
    const previousSessionId = previousSelectedSessionIdRef.current;
    if (previousSessionId && previousSessionId !== selectedSessionId) {
      // Save current view state before switching
      sessionViewStateRef.current.set(previousSessionId, currentViewStateRef.current);
    }

    previousSelectedSessionIdRef.current = selectedSessionId;
    selectedSessionIdRef.current = selectedSessionId;

    if (typeof window !== "undefined") {
      if (selectedSessionId) {
        window.sessionStorage.setItem(CHAT_SELECTED_SESSION_STORAGE_KEY, selectedSessionId);
      } else {
        window.sessionStorage.removeItem(CHAT_SELECTED_SESSION_STORAGE_KEY);
      }
    }
  }, [selectedSessionId]);

  // History loading logic
  useEffect(() => {
    if (!canAccessProtectedApi || !selectedSessionId) {
      const emptyState: SessionViewState = {
        messages: [],
        activeTools: [],
        recentTools: [],
        pendingAssistantId: null,
        streamPhase: "idle",
      };
      syncCurrentViewState(emptyState);
      setMessages([]);
      setActiveTools([]);
      setRecentTools([]);
      setPendingAssistantId(null);
      setStreamPhase("idle");
      return;
    }

    const sessionId = selectedSessionId;
    const cachedViewState = sessionViewStateRef.current.get(sessionId);
    let cancelled = false;

    if (cachedViewState) {
      syncCurrentViewState(cachedViewState);
      setMessages(cachedViewState.messages);
      setActiveTools(cachedViewState.activeTools);
      setRecentTools(cachedViewState.recentTools);
      setPendingAssistantId(cachedViewState.pendingAssistantId);
      setStreamPhase(cachedViewState.streamPhase);
    } else {
      const emptyState: SessionViewState = {
        messages: [],
        activeTools: [],
        recentTools: [],
        pendingAssistantId: null,
        streamPhase: "idle",
      };
      syncCurrentViewState(emptyState);
      setMessages([]);
      setActiveTools([]);
      setRecentTools([]);
      setPendingAssistantId(null);
      setStreamPhase("idle");
    }

    async function loadHistory() {
      setIsLoadingHistory(true);
      setError(null);
      try {
        const loadedMessages = await chatApi.loadHistory(sessionId);
        if (cancelled) return;

        const currentCached = sessionViewStateRef.current.get(sessionId);
        const hasTransientState =
            !!currentCached?.pendingAssistantId ||
            ["connecting", "thinking", "streaming"].includes(currentCached?.streamPhase ?? "") ||
            (currentCached?.activeTools.length ?? 0) > 0;

        const nextState: SessionViewState = hasTransientState
            ? {
                messages: (currentCached && currentCached.messages.length > 0) ? currentCached.messages : loadedMessages,
                activeTools: currentCached?.activeTools ?? [],
                recentTools: currentCached?.recentTools ?? [],
                pendingAssistantId: currentCached?.pendingAssistantId ?? null,
                streamPhase: currentCached?.streamPhase ?? "idle",
              }
            : {
                messages: loadedMessages,
                activeTools: [],
                recentTools: currentCached?.recentTools ?? [],
                pendingAssistantId: null,
                streamPhase: "idle",
              };

        sessionViewStateRef.current.set(sessionId, nextState);
        if (selectedSessionIdRef.current === sessionId) {
          syncCurrentViewState(nextState);
          setMessages(nextState.messages);
          setActiveTools(nextState.activeTools);
          setRecentTools(nextState.recentTools);
          setPendingAssistantId(nextState.pendingAssistantId);
          setStreamPhase(nextState.streamPhase);
        }
      } catch (loadError) {
        if (!cancelled) {
          if (loadError instanceof ApiRequestError && loadError.status === 401) {
            markUnauthorized();
          } else {
            setError(loadError instanceof Error ? loadError.message : "Failed to load history");
          }
        }
      } finally {
        if (!cancelled) setIsLoadingHistory(false);
      }
    }

    void loadHistory();
    return () => {
      cancelled = true;
    };
  }, [canAccessProtectedApi, selectedSessionId, markUnauthorized, t]);

  const deleteSession = useCallback(async (sessionId: string) => {
    if (deletingSessionId || !canAccessProtectedApi) return;

    setDeletingSessionId(sessionId);
    setError(null);

    try {
      await chatApi.deleteSession(sessionId);
      const remainingSessions = sessions.filter((s) => s.id !== sessionId);
      setSessions(remainingSessions);
      sessionViewStateRef.current.delete(sessionId);

      if (selectedSessionId === sessionId) {
        const nextSessionId = remainingSessions[0]?.id ?? null;
        setSelectedSessionId(nextSessionId);
        if (!nextSessionId) {
          setMessages([]);
          setActiveTools([]);
          setRecentTools([]);
        }
      }
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 401) {
        markUnauthorized();
      } else {
        setError(err instanceof Error ? err.message : "Failed to delete session");
      }
    } finally {
      setDeletingSessionId(null);
    }
  }, [deletingSessionId, canAccessProtectedApi, sessions, selectedSessionId, markUnauthorized]);

  const removeSession = useCallback((sessionId: string) => {
    setSessions((current) => current.filter((session) => session.id !== sessionId));
    sessionViewStateRef.current.delete(sessionId);

    if (selectedSessionIdRef.current === sessionId) {
      setSelectedSessionId(null);
      const emptyState: SessionViewState = {
        messages: [],
        activeTools: [],
        recentTools: [],
        pendingAssistantId: null,
        streamPhase: "idle",
      };
      syncCurrentViewState(emptyState);
      setMessages([]);
      setActiveTools([]);
      setRecentTools([]);
      setPendingAssistantId(null);
      setStreamPhase("idle");
    }
  }, [syncCurrentViewState]);

  return {
    sessions,
    selectedSessionId,
    messages,
    activeTools,
    recentTools,
    pendingAssistantId,
    streamPhase,
    isLoadingSessions,
    isLoadingHistory,
    deletingSessionId,
    error,
    setError,
    setMessages,
    setActiveTools,
    setRecentTools,
    setPendingAssistantId,
    setStreamPhase,
    selectSession,
    upsertSession,
    removeSession,
    refreshSessions,
    deleteSession,
    updateSessionViewState,
    sessionViewStateRef,
  };
}
