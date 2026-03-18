import { Plus, SendHorizontal, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ReactNode } from "react";
import { Panel } from "../../../components/surfaces/Panel";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { ApiRequestError } from "../../../lib/api/client";
import { dashboardApi } from "../../dashboard/api";
import {
  chatApi,
  type ChatMessage,
  type ChatSessionSummary,
  type ChatTurnStreamEvent,
} from "../api";

interface ActiveToolStatus {
  toolId: string;
  label: string;
  status: "running" | "ok" | "error";
}

type StreamPhase = "idle" | "connecting" | "thinking" | "streaming";

function renderInlineBreaks(text: string): ReactNode[] {
  return text.split("\n").flatMap((line, index, lines) => {
    const nodes: ReactNode[] = [line];
    if (index < lines.length - 1) {
      nodes.push(<br key={`br-${index}`} />);
    }
    return nodes;
  });
}

function renderMessageContent(content: string): ReactNode[] {
  const normalized = content.replace(/\r\n/g, "\n").trim();
  if (!normalized) {
    return [];
  }

  const lines = normalized.split("\n");
  const blocks: ReactNode[] = [];
  let paragraphLines: string[] = [];
  let listItems: string[] = [];

  function flushParagraph() {
    if (paragraphLines.length === 0) {
      return;
    }
    const text = paragraphLines.join("\n").trim();
    if (text) {
      blocks.push(<p key={`block-${blocks.length}`}>{renderInlineBreaks(text)}</p>);
    }
    paragraphLines = [];
  }

  function flushList() {
    if (listItems.length === 0) {
      return;
    }
    blocks.push(
      <ul key={`block-${blocks.length}`}>
        {listItems.map((item, itemIndex) => (
          <li key={`item-${itemIndex}`}>{item}</li>
        ))}
      </ul>,
    );
    listItems = [];
  }

  for (const rawLine of lines) {
    const line = rawLine.trim();

    if (!line) {
      flushParagraph();
      flushList();
      continue;
    }

    const headingMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headingMatch) {
      flushParagraph();
      flushList();
      const level = headingMatch[1].length;
      const title = headingMatch[2];
      if (level === 1) {
        blocks.push(<h1 key={`block-${blocks.length}`}>{title}</h1>);
      } else if (level === 2) {
        blocks.push(<h2 key={`block-${blocks.length}`}>{title}</h2>);
      } else {
        blocks.push(<h3 key={`block-${blocks.length}`}>{title}</h3>);
      }
      continue;
    }

    if (/^[-*]\s+/.test(line)) {
      flushParagraph();
      listItems.push(line.replace(/^[-*]\s+/, ""));
      continue;
    }

    flushList();
    paragraphLines.push(line);
  }

  flushParagraph();
  flushList();
  return blocks;
}

function extractErrorHost(message: string): string | null {
  const match = message.match(/https?:\/\/([^/\s)]+)/i);
  return match?.[1] ?? null;
}

function toFriendlyChatError(
  error: unknown,
  t: ReturnType<typeof useTranslation>["t"],
  markUnauthorized: () => void,
): string {
  if (error instanceof ApiRequestError && error.status === 401) {
    markUnauthorized();
    return t("auth.invalidBody");
  }

  const rawMessage =
    error instanceof Error ? error.message : "Failed to send message";

  if (rawMessage.includes("transport_failure")) {
    const host = extractErrorHost(rawMessage);
    return t("chat.errors.transportFailure", {
      host: host ?? t("chat.errors.providerHostFallback"),
    });
  }

  return rawMessage;
}

export default function ChatPage() {
  const { t } = useTranslation();
  const { canAccessProtectedApi, authRevision, markUnauthorized, status } =
    useWebConnection();
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [composerText, setComposerText] = useState("");
  const [isLoadingSessions, setIsLoadingSessions] = useState(true);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [deletingSessionId, setDeletingSessionId] = useState<string | null>(null);
  const [activeTools, setActiveTools] = useState<ActiveToolStatus[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [memoryWindow, setMemoryWindow] = useState<number | null>(null);
  const [pendingAssistantId, setPendingAssistantId] = useState<string | null>(null);
  const [streamPhase, setStreamPhase] = useState<StreamPhase>("idle");
  const [loadingLabelIndex, setLoadingLabelIndex] = useState(0);

  const loadingPhraseKeys = useMemo(() => {
    switch (streamPhase) {
      case "connecting":
        return [
          "chat.loading.connectingA",
          "chat.loading.connectingB",
          "chat.loading.connectingC",
        ];
      case "thinking":
        return [
          "chat.loading.thinkingA",
          "chat.loading.thinkingB",
          "chat.loading.thinkingC",
        ];
      case "streaming":
        return [
          "chat.loading.streamingA",
          "chat.loading.streamingB",
          "chat.loading.streamingC",
        ];
      default:
        return [];
    }
  }, [streamPhase]);

  const loadingPhrases = useMemo(
    () => loadingPhraseKeys.map((key) => t(key)),
    [loadingPhraseKeys, t],
  );
  const loadingLabelBase =
    loadingPhrases.length > 0
      ? loadingPhrases[loadingLabelIndex % loadingPhrases.length]
      : t("chat.generating");
  useEffect(() => {
    if (streamPhase === "idle" || loadingPhrases.length <= 1) {
      setLoadingLabelIndex(0);
      return;
    }

    const intervalId = window.setInterval(() => {
      setLoadingLabelIndex((current) => (current + 1) % loadingPhrases.length);
    }, 2200);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [loadingPhrases.length, streamPhase]);

  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      setSessions([]);
      setSelectedSessionId(null);
      setMessages([]);
      setActiveTools([]);
      setMemoryWindow(null);
      setIsLoadingSessions(false);
      setError(
        status === "unauthorized"
          ? t("auth.invalidBody")
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
        const loadedSessions = await chatApi.listSessions();
        if (cancelled) {
          return;
        }
        setSessions(loadedSessions);
        setSelectedSessionId((current) => current ?? loadedSessions[0]?.id ?? null);
      } catch (loadError) {
        if (!cancelled) {
          if (loadError instanceof ApiRequestError && loadError.status === 401) {
            markUnauthorized();
            setError(t("auth.invalidBody"));
          } else {
            setError(loadError instanceof Error ? loadError.message : "Failed to load sessions");
          }
        }
      } finally {
        if (!cancelled) {
          setIsLoadingSessions(false);
        }
      }
    }

    void loadSessions();

    return () => {
      cancelled = true;
    };
  }, [authRevision, canAccessProtectedApi, markUnauthorized, status, t]);

  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      setMemoryWindow(null);
      return () => {
        cancelled = true;
      };
    }

    async function loadConfigSnapshot() {
      try {
        const config = await dashboardApi.loadConfig();
        if (!cancelled) {
          setMemoryWindow(config.slidingWindow);
        }
      } catch (loadError) {
        if (!cancelled && loadError instanceof ApiRequestError && loadError.status === 401) {
          markUnauthorized();
        }
      }
    }

    void loadConfigSnapshot();

    return () => {
      cancelled = true;
    };
  }, [authRevision, canAccessProtectedApi, markUnauthorized]);

  useEffect(() => {
    if (!canAccessProtectedApi) {
      setMessages([]);
      setActiveTools([]);
      return;
    }

    if (!selectedSessionId) {
      return;
    }

    const sessionId = selectedSessionId;
    let cancelled = false;

    async function loadHistory() {
      setIsLoadingHistory(true);
      setError(null);
      try {
        const loadedMessages = await chatApi.loadHistory(sessionId);
        if (!cancelled) {
          setMessages(loadedMessages);
          setActiveTools([]);
        }
      } catch (loadError) {
        if (!cancelled) {
          if (loadError instanceof ApiRequestError && loadError.status === 401) {
            markUnauthorized();
            setError(t("auth.invalidBody"));
          } else {
            setError(loadError instanceof Error ? loadError.message : "Failed to load history");
          }
          setMessages([]);
          setActiveTools([]);
        }
      } finally {
        if (!cancelled) {
          setIsLoadingHistory(false);
        }
      }
    }

    void loadHistory();

    return () => {
      cancelled = true;
    };
  }, [canAccessProtectedApi, markUnauthorized, selectedSessionId, t]);

  async function refreshSessions(preferredSessionId?: string) {
    const loadedSessions = await chatApi.listSessions();
    setSessions(loadedSessions);
    setSelectedSessionId(
      (current) => preferredSessionId ?? current ?? loadedSessions[0]?.id ?? null,
    );
  }

  function handleStreamEvent(
    event: ChatTurnStreamEvent,
    placeholderId: string,
  ) {
    switch (event.type) {
      case "turn.started":
        setStreamPhase("thinking");
        break;
      case "message.delta":
        setStreamPhase("streaming");
        setMessages((current) =>
          current.map((message) =>
            message.id === placeholderId
              ? { ...message, content: `${message.content}${event.delta}` }
              : message,
          ),
        );
        break;
      case "tool.started":
        setStreamPhase((current) =>
          current === "connecting" ? "thinking" : current,
        );
        setActiveTools((current) => {
          const existing = current.find((item) => item.toolId === event.toolId);
          if (existing) {
            return current.map((item) =>
              item.toolId === event.toolId
                ? { ...item, label: event.label, status: "running" }
                : item,
            );
          }
          return [
            ...current,
            { toolId: event.toolId, label: event.label, status: "running" },
          ];
        });
        break;
      case "tool.finished":
        setActiveTools((current) =>
          current.map((item) =>
            item.toolId === event.toolId
              ? {
                  ...item,
                  label: event.label,
                  status: event.outcome === "ok" ? "ok" : "error",
                }
              : item,
          ),
        );
        break;
      case "turn.completed":
        setStreamPhase("idle");
        setPendingAssistantId(null);
        setMessages((current) =>
          current.map((message) =>
            message.id === placeholderId ? event.message : message,
          ),
        );
        break;
      case "turn.failed":
        setStreamPhase("idle");
        setPendingAssistantId(null);
        setMessages((current) =>
          current.filter((message) => message.id !== placeholderId),
        );
        setError(event.message);
        break;
      default:
        break;
    }
  }

  async function handleSubmit() {
    const input = composerText.trim();
    if (!input || isSubmitting || !canAccessProtectedApi) {
      return;
    }

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
    const previousMessages = messages;
    const previousTools = activeTools;

    setError(null);
    setIsSubmitting(true);
    setStreamPhase("connecting");
    setPendingAssistantId(placeholderAssistantId);
    setActiveTools([]);
    setMessages((current) => [
      ...current,
      optimisticUserMessage,
      placeholderAssistantMessage,
    ]);
    setComposerText("");

    try {
      const targetSessionId =
        selectedSessionId ?? (await chatApi.createSession(input.slice(0, 48)));
      const acceptedTurn = await chatApi.createTurn(targetSessionId, input);

      await chatApi.streamTurn(targetSessionId, acceptedTurn.turnId, {
        onEvent: (event) => {
          handleStreamEvent(event, placeholderAssistantId);
        },
      });

      setActiveTools([]);
      await refreshSessions(targetSessionId);
    } catch (submitError) {
      setStreamPhase("idle");
      setPendingAssistantId(null);
      setMessages(previousMessages);
      setActiveTools(previousTools);
      setError(toFriendlyChatError(submitError, t, markUnauthorized));
      setComposerText(input);
    } finally {
      setStreamPhase("idle");
      setIsSubmitting(false);
    }
  }

  async function handleDeleteSession(sessionId: string) {
    if (deletingSessionId || !canAccessProtectedApi) {
      return;
    }

    setDeletingSessionId(sessionId);
    setError(null);

    try {
      await chatApi.deleteSession(sessionId);
      const remainingSessions = sessions.filter((session) => session.id !== sessionId);
      setSessions(remainingSessions);

      if (selectedSessionId === sessionId) {
        const nextSessionId = remainingSessions[0]?.id ?? null;
        setSelectedSessionId(nextSessionId);
        if (!nextSessionId) {
          setMessages([]);
          setActiveTools([]);
        }
      }
    } catch (deleteError) {
      if (deleteError instanceof ApiRequestError && deleteError.status === 401) {
        markUnauthorized();
        setError(t("auth.invalidBody"));
      } else {
        setError(deleteError instanceof Error ? deleteError.message : "Failed to delete session");
      }
    } finally {
      setDeletingSessionId(null);
    }
  }

  const selectedSession =
    sessions.find((session) => session.id === selectedSessionId) ?? sessions[0] ?? null;

  return (
    <div className="page page-chat">
      <div className="chat-shell">
        <Panel
          eyebrow={t("chat.panels.sessions")}
          title={t("chat.newSession")}
          aside={
            <button
              type="button"
              className="panel-action"
              onClick={() => {
                setSelectedSessionId(null);
                setMessages([]);
                setActiveTools([]);
                setError(null);
              }}
              disabled={!canAccessProtectedApi}
            >
              <Plus size={14} />
            </button>
          }
        >
          <div className="stack-list stack-list-scroll">
            {isLoadingSessions ? (
              <div className="empty-state">Loading sessions...</div>
            ) : sessions.length > 0 ? (
              sessions.map((session) => (
                <div
                  key={session.id}
                  className={`session-item${session.id === selectedSessionId ? " is-selected" : ""}`}
                >
                  <button
                    type="button"
                    className="session-select"
                    onClick={() => {
                      setSelectedSessionId(session.id);
                    }}
                  >
                    <span>{session.title}</span>
                    <span className="session-meta">{session.updatedAt}</span>
                  </button>
                  <button
                    type="button"
                    className="session-delete"
                    aria-label={`${t("chat.deleteSession")} ${session.title}`}
                    onClick={() => {
                      void handleDeleteSession(session.id);
                    }}
                    disabled={deletingSessionId === session.id}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))
            ) : (
              <div className="empty-state">No saved sessions yet.</div>
            )}
          </div>
        </Panel>

        <Panel title={t("chat.title")} className="panel-chat-main" hideHeader>
          <div className="chat-main">
            <div className="chat-topline">
              <div>
                <div className="panel-eyebrow">{t("chat.eyebrow")}</div>
                <div className="chat-session-title">
                  {selectedSession?.title ?? "Untitled session"}
                </div>
              </div>
              <div className="chat-topline-meta">
                <span>{selectedSession?.updatedAt ?? "No history"}</span>
                <span
                  className="chat-status-dot"
                  title={t("status.connected")}
                  aria-label={t("status.connected")}
                />
              </div>
            </div>

            <div className="message-list">
              {error ? <div className="empty-state">{error}</div> : null}
              {!error && isLoadingHistory ? (
                <div className="empty-state">Loading history...</div>
              ) : null}
              {!error && !isLoadingHistory && messages.length === 0 ? (
                <div className="empty-state">
                  Start a new conversation or open an existing session.
                </div>
              ) : null}
              {!error &&
                !isLoadingHistory &&
                messages.map((message) => (
                  <article
                    key={message.id}
                    className={`message-bubble message-bubble-${message.role}`}
                  >
                    <div className="message-role">{message.role}</div>
                    {isSubmitting &&
                    message.role === "assistant" &&
                    message.id === pendingAssistantId ? (
                      <div className="chat-loading-inline" aria-live="polite">
                        <span>
                          {loadingLabelBase}
                          <span className="chat-loading-ellipsis" aria-hidden="true" />
                        </span>
                      </div>
                    ) : null}
                    {message.content ? (
                      <div className="message-content">
                        {renderMessageContent(message.content)}
                      </div>
                    ) : null}
                  </article>
                ))}
            </div>

            {activeTools.length > 0 ? (
              <div className="chat-stream-tools">
                {activeTools.map((tool) => (
                  <div key={tool.toolId} className={`chat-tool-chip chat-tool-chip-${tool.status}`}>
                    <span className="chat-tool-chip-label">{tool.label}</span>
                    <strong>{t(`chat.toolStatus.${tool.status}`)}</strong>
                  </div>
                ))}
              </div>
            ) : null}

            <form
              className="composer composer-inline"
              onSubmit={(event) => {
                event.preventDefault();
                void handleSubmit();
              }}
            >
              <div className="composer-shell">
                <textarea
                  className="composer-input"
                  rows={3}
                  placeholder={t("chat.inputPlaceholder")}
                  value={composerText}
                  onChange={(event) => {
                    setComposerText(event.target.value);
                  }}
                  disabled={isSubmitting || !canAccessProtectedApi}
                />
                {deletingSessionId ? (
                  <div className="composer-hint">{t("chat.deleting")}</div>
                ) : null}
                <button
                  type="submit"
                  className="composer-submit"
                  disabled={
                    isSubmitting || !composerText.trim() || !canAccessProtectedApi
                  }
                >
                  <SendHorizontal size={16} />
                  <span className="sr-only">{isSubmitting ? "Sending..." : t("chat.send")}</span>
                </button>
              </div>
            </form>
          </div>
        </Panel>

        <Panel eyebrow={t("chat.panels.inspector")} title={t("status.local")}>
            <div className="metric-grid metric-grid-scroll">
              <div className="metric-card">
                <span className="metric-label">{t("status.providerReady")}</span>
                <strong>{selectedSession ? "Live session loaded" : "Waiting for session"}</strong>
              </div>
              <div className="metric-card">
                <span className="metric-label">{t("status.memoryHealthy")}</span>
                <strong>{messages.length > 0 ? `${messages.length} messages` : "No history"}</strong>
              </div>
              <div className="metric-card">
                <span className="metric-label">{t("chat.memoryWindow.label")}</span>
                <strong>
                  {memoryWindow !== null
                    ? t("chat.memoryWindow.value", { count: memoryWindow })
                    : t("chat.memoryWindow.pending")}
                </strong>
                <span>{t("chat.memoryWindow.hint")}</span>
              </div>
            </div>
          </Panel>
        </div>
    </div>
  );
}
