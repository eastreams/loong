import { Copy, Plus, SendHorizontal, ThumbsDown, ThumbsUp, Trash2 } from "lucide-react";
import { Suspense, lazy, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import TextareaAutosize from "react-textarea-autosize";
import "../../../styles/chat.css";
import { Panel } from "../../../components/surfaces/Panel";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { ApiRequestError } from "../../../lib/api/client";
import { dashboardApi } from "../../dashboard/api";
import { useChatSessions } from "../hooks/useChatSessions";
import { useChatStream } from "../hooks/useChatStream";
import { CopyButton } from "../../../components/feedback/CopyButton";

const MarkdownBlock = lazy(async () => {
  const module = await import("../components/MarkdownBlock");
  return { default: module.MarkdownBlock };
});

export default function ChatPage() {
  const { t } = useTranslation();
  const connection = useWebConnection();
  const { canAccessProtectedApi, authRevision, markUnauthorized } = connection;

  const [composerText, setComposerText] = useState("");
  const [memoryWindow, setMemoryWindow] = useState<number | null>(null);
  const [currentModel, setCurrentModel] = useState("");
  const [currentProvider, setCurrentProvider] = useState<string | null>(null);
  const [loadingLabelIndex, setLoadingLabelIndex] = useState(0);
  const messageListRef = useRef<HTMLDivElement | null>(null);
  const messagesEndRef = useRef<HTMLDivElement | null>(null);
  const shouldAutoScrollRef = useRef(true);

  const sessionsState = useChatSessions(t);
  const {
    sessions,
    selectedSessionId,
    messages,
    activeTools,
    pendingAssistantId,
    streamPhase,
    isLoadingSessions,
    isLoadingHistory,
    deletingSessionId,
    error,
    setError,
    selectSession,
    upsertSession,
    refreshSessions,
    deleteSession,
    updateSessionViewState,
  } = sessionsState;

  const streamState = useChatStream({
    t,
    sessionId: selectedSessionId,
    canAccessProtectedApi,
    markUnauthorized,
    authMode: connection.authMode,
    tokenPath: connection.tokenPath,
    tokenEnv: connection.tokenEnv,
    updateSessionViewState,
    selectSession,
    upsertSession,
    removeSession: sessionsState.removeSession,
    refreshSessions,
    setError,
  });

  const { isSubmitting, sendMessage, stopStream } = streamState;

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
    async function loadConfigSnapshot() {
      try {
        const config = await dashboardApi.loadConfig();
        if (!cancelled) {
          setMemoryWindow(config.slidingWindow);
          setCurrentModel(config.model);
          setCurrentProvider(config.activeProvider);
        }
      } catch (loadError) {
        if (!cancelled && loadError instanceof ApiRequestError && loadError.status === 401) {
          markUnauthorized();
        }
      }
    }
    if (canAccessProtectedApi) void loadConfigSnapshot();
    return () => { cancelled = true; };
  }, [authRevision, canAccessProtectedApi, markUnauthorized]);

  useEffect(() => {
    shouldAutoScrollRef.current = true;
  }, [selectedSessionId]);

  useEffect(() => {
    if (!messageListRef.current || !shouldAutoScrollRef.current) return;
    if (messagesEndRef.current) {
      messagesEndRef.current.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [messages, isLoadingHistory, selectedSessionId]);

  async function handleSubmit() {
    const input = composerText.trim();
    if (!input) return;
    setComposerText("");
    const accepted = await sendMessage(input);
    if (accepted === false) {
      setComposerText(input);
    }
  }

  const selectedSession =
    sessions.find((session) => session.id === selectedSessionId) ?? null;

  function renderMessageFallback(content: string) {
    return (
      <div className="message-content">
        <pre className="message-markdown-fallback">{content}</pre>
      </div>
    );
  }

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
                selectSession(null);
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
              <div className="empty-state">{t("chat.loadingSessions")}</div>
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
                      selectSession(session.id);
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
                      void deleteSession(session.id);
                    }}
                    disabled={deletingSessionId === session.id}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))
            ) : (
              <div className="empty-state">{t("chat.noSessions")}</div>
            )}
          </div>
        </Panel>

        <Panel title={t("chat.title")} className="panel-chat-main" hideHeader>
          <div className="chat-main">
            <div className="chat-topline">
              <div>
                <div className="panel-eyebrow">{t("chat.eyebrow")}</div>
                <div className="chat-session-title">
                  {selectedSession?.title ?? t("chat.untitledSession")}
                </div>
              </div>
              <div className="chat-topline-meta">
                <span>{selectedSession?.updatedAt ?? t("chat.noHistory")}</span>
                <span
                  className="chat-status-dot"
                  title={t("status.connected")}
                  aria-label={t("status.connected")}
                />
              </div>
            </div>

            <div
              className="message-list"
              ref={messageListRef}
              onScroll={(event) => {
                const target = event.currentTarget;
                const distanceToBottom =
                  target.scrollHeight - target.scrollTop - target.clientHeight;
                shouldAutoScrollRef.current = distanceToBottom < 80;
              }}
            >
              {error && messages.length === 0 ? (
                <div className="empty-state">{error}</div>
              ) : null}
              {!error && isLoadingHistory && messages.length === 0 ? (
                <div className="empty-state">{t("chat.loadingHistory")}</div>
              ) : null}
              {!error && !isLoadingHistory && messages.length === 0 ? (
                <div className="empty-state">{t("chat.emptyState")}</div>
              ) : null}
              {error && messages.length > 0 ? (
                <div className="empty-state" style={{ marginBottom: "1rem" }}>
                  {error}
                </div>
              ) : null}
              {messages.map((message) => (
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
                        <Suspense fallback={renderMessageFallback(message.content)}>
                          <MarkdownBlock content={message.content} />
                        </Suspense>
                        {message.role === "assistant" && (
                          <div className="message-actions">
                            <CopyButton
                              className="message-action-btn"
                              title={t("chat.actions.copy")}
                              text={message.content}
                            />
                            <button
                              type="button"
                              className="message-action-btn"
                              title={t("chat.actions.good")}
                            >
                              <ThumbsUp size={14} />
                            </button>
                            <button
                              type="button"
                              className="message-action-btn"
                              title={t("chat.actions.bad")}
                            >
                              <ThumbsDown size={14} />
                            </button>
                          </div>
                        )}
                      </div>
                    ) : null}
                  </article>
                ))}
              <div ref={messagesEndRef} style={{ height: 1 }} />
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
              <div className="composer-shell" style={{ alignItems: 'flex-end' }}>
                <TextareaAutosize
                  className="composer-input"
                  minRows={1}
                  maxRows={8}
                  placeholder={t("chat.inputPlaceholder")}
                  value={composerText}
                  onChange={(event) => {
                    setComposerText(event.target.value);
                  }}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Enter" &&
                      !event.shiftKey &&
                      !event.nativeEvent.isComposing
                    ) {
                      event.preventDefault();
                      void handleSubmit();
                    }
                  }}
                  disabled={isSubmitting || !canAccessProtectedApi}
                  style={{ resize: "none" }}
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
                <strong>{selectedSession ? t("chat.inspector.sessionLoaded") : t("chat.inspector.waitingForSession")}</strong>
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
              </div>
              <div className="metric-card">
                <span className="metric-label">{t("chat.currentModel.label")}</span>
                <strong title={currentModel || t("chat.currentModel.pending")}>
                  {currentModel || t("chat.currentModel.pending")}
                </strong>
                <span>
                  {currentProvider
                    ? t("chat.currentModel.provider", { provider: currentProvider })
                    : t("chat.currentModel.providerPending")}
                </span>
              </div>
            </div>
          </Panel>
        </div>
    </div>
  );
}
