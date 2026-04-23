import {
  Check,
  Copy,
  PencilLine,
  Plus,
  SendHorizontal,
  ThumbsDown,
  ThumbsUp,
  Trash2,
  X,
} from "lucide-react";
import { Suspense, lazy, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import TextareaAutosize from "react-textarea-autosize";
import "../../../styles/chat.css";
import { CopyButton } from "../../../components/feedback/CopyButton";
import { Panel } from "../../../components/surfaces/Panel";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { ApiRequestError } from "../../../lib/api/client";
import { dashboardApi } from "../../dashboard/api";
import { ChatMascot } from "../components/ChatMascot";
import { useChatSessions } from "../hooks/useChatSessions";
import { useChatStream } from "../hooks/useChatStream";
import {
  CHAT_MASCOT_TOGGLED_EVENT,
  readChatMascotEnabled,
} from "../mascotPreference";

const MarkdownBlock = lazy(async () => {
  const module = await import("../components/MarkdownBlock");
  return { default: module.MarkdownBlock };
});

const CHAT_SESSION_TITLE_OVERRIDES_STORAGE_KEY = "loongclaw.web.chat.sessionTitleOverrides";

function readStoredSessionTitleOverrides(): Record<string, string> {
  if (typeof window === "undefined") {
    return {};
  }

  try {
    const raw = window.localStorage.getItem(CHAT_SESSION_TITLE_OVERRIDES_STORAGE_KEY);
    if (!raw) {
      return {};
    }

    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" ? (parsed as Record<string, string>) : {};
  } catch {
    return {};
  }
}

export default function ChatPage() {
  const { t, i18n } = useTranslation();
  const isChinese = i18n.language.startsWith("zh");
  const renameSessionLabel = isChinese ? "重命名会话" : "Rename session";
  const saveSessionNameLabel = isChinese ? "保存会话名" : "Save session name";
  const cancelRenameLabel = isChinese ? "取消重命名" : "Cancel rename";

  const connection = useWebConnection();
  const { canAccessProtectedApi, authRevision, markUnauthorized } = connection;

  const [composerText, setComposerText] = useState("");
  const [memoryWindow, setMemoryWindow] = useState<number | null>(null);
  const [currentModel, setCurrentModel] = useState("");
  const [loadingLabelIndex, setLoadingLabelIndex] = useState(0);
  const [sessionTitleOverrides, setSessionTitleOverrides] = useState<Record<string, string>>(
    () => readStoredSessionTitleOverrides(),
  );
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [showMascot, setShowMascot] = useState(() => readChatMascotEnabled());
  const messageListRef = useRef<HTMLDivElement | null>(null);
  const messagesEndRef = useRef<HTMLDivElement | null>(null);
  const shouldAutoScrollRef = useRef(true);

  const sessionsState = useChatSessions(t);
  const {
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

  const { isSubmitting, sendMessage } = streamState;

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
        }
      } catch (loadError) {
        if (!cancelled && loadError instanceof ApiRequestError && loadError.status === 401) {
          markUnauthorized();
        }
      }
    }

    if (canAccessProtectedApi) {
      void loadConfigSnapshot();
    }

    return () => {
      cancelled = true;
    };
  }, [authRevision, canAccessProtectedApi, markUnauthorized]);

  useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    window.localStorage.setItem(
      CHAT_SESSION_TITLE_OVERRIDES_STORAGE_KEY,
      JSON.stringify(sessionTitleOverrides),
    );
  }, [sessionTitleOverrides]);

  useEffect(() => {
    function handleStorage(event: StorageEvent) {
      if (event.key) {
        setShowMascot(readChatMascotEnabled());
      }
    }

    function handleMascotToggled() {
      setShowMascot(readChatMascotEnabled());
    }

    window.addEventListener("storage", handleStorage);
    window.addEventListener(CHAT_MASCOT_TOGGLED_EVENT, handleMascotToggled);
    return () => {
      window.removeEventListener("storage", handleStorage);
      window.removeEventListener(CHAT_MASCOT_TOGGLED_EVENT, handleMascotToggled);
    };
  }, []);

  useEffect(() => {
    if (sessions.length === 0) {
      return;
    }

    setSessionTitleOverrides((current) => {
      const liveIds = new Set(sessions.map((session) => session.id));
      const next = Object.fromEntries(
        Object.entries(current).filter(([sessionId]) => liveIds.has(sessionId)),
      );
      return Object.keys(next).length === Object.keys(current).length ? current : next;
    });
  }, [sessions]);

  useEffect(() => {
    shouldAutoScrollRef.current = true;
  }, [selectedSessionId]);

  useEffect(() => {
    if (!messageListRef.current || !shouldAutoScrollRef.current) {
      return;
    }

    if (messagesEndRef.current) {
      messagesEndRef.current.scrollIntoView({ behavior: "smooth", block: "end" });
    }
  }, [messages, isLoadingHistory, selectedSessionId]);

  async function handleSubmit() {
    const input = composerText.trim();
    if (!input) {
      return;
    }

    setComposerText("");
    const accepted = await sendMessage(input);
    if (accepted === false) {
      setComposerText(input);
    }
  }

  function resolveSessionTitle(sessionId: string, fallbackTitle: string) {
    const override = sessionTitleOverrides[sessionId]?.trim();
    return override && override.length > 0 ? override : fallbackTitle;
  }

  function beginRename(sessionId: string, fallbackTitle: string) {
    setRenamingSessionId(sessionId);
    setRenameDraft(resolveSessionTitle(sessionId, fallbackTitle));
  }

  function cancelRename() {
    setRenamingSessionId(null);
    setRenameDraft("");
  }

  function saveRename(sessionId: string, fallbackTitle: string) {
    const trimmed = renameDraft.trim();

    setSessionTitleOverrides((current) => {
      const next = { ...current };
      if (!trimmed || trimmed === fallbackTitle.trim()) {
        delete next[sessionId];
      } else {
        next[sessionId] = trimmed;
      }
      return next;
    });

    cancelRename();
  }

  function removeSessionTitleOverride(sessionId: string) {
    setSessionTitleOverrides((current) => {
      if (!(sessionId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[sessionId];
      return next;
    });
  }

  function renderMessageFallback(content: string) {
    return (
      <div className="message-content">
        <pre className="message-markdown-fallback">{content}</pre>
      </div>
    );
  }

  function formatRecentToolTime(value: string) {
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) {
      return value;
    }

    return parsed.toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  }

  function formatSessionActivityTime(value: string) {
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) {
      return value;
    }

    const diffMs = parsed.getTime() - Date.now();
    const absMinutes = Math.abs(diffMs) / 60_000;
    const locale = isChinese ? "zh-CN" : "en";
    const formatter = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });

    if (absMinutes < 1) {
      return isChinese ? "刚刚" : "just now";
    }

    if (absMinutes < 60) {
      return formatter.format(Math.round(diffMs / 60_000), "minute");
    }

    const absHours = absMinutes / 60;
    if (absHours < 24) {
      return formatter.format(Math.round(diffMs / 3_600_000), "hour");
    }

    const absDays = absHours / 24;
    if (absDays < 7) {
      return formatter.format(Math.round(diffMs / 86_400_000), "day");
    }

    return parsed.toLocaleString(locale, {
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function resolveRecentToolStatusTone(tool: (typeof recentTools)[number]) {
    if (tool.status === "error" && /timeout/i.test(tool.detail ?? "")) {
      return "warning";
    }
    return tool.status;
  }

  function resolveRecentToolDetail(tool: (typeof recentTools)[number]) {
    const genericDetails = new Set([
      t("chat.recentTools.detail.ok"),
      t("chat.recentTools.detail.error"),
    ]);

    if (!tool.detail || genericDetails.has(tool.detail)) {
      return null;
    }

    return tool.detail;
  }

  const selectedSession =
    sessions.find((session) => session.id === selectedSessionId) ?? null;
  const selectedSessionDisplayTitle = selectedSession
    ? resolveSessionTitle(selectedSession.id, selectedSession.title)
    : null;

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
              sessions.map((session) => {
                const displayTitle = resolveSessionTitle(session.id, session.title);
                const isRenaming = renamingSessionId === session.id;
                const isGenerating =
                  session.id === selectedSessionId && streamPhase !== "idle";

                return (
                  <div
                    key={session.id}
                    className={`session-item${session.id === selectedSessionId ? " is-selected" : ""}`}
                  >
                    <button
                      type="button"
                      className="session-select"
                      onClick={() => {
                        if (!isRenaming) {
                          selectSession(session.id);
                        }
                      }}
                      disabled={isRenaming}
                    >
                      {isRenaming ? (
                        <input
                          className="session-rename-input"
                          value={renameDraft}
                          onChange={(event) => {
                            setRenameDraft(event.target.value);
                          }}
                          onClick={(event) => {
                            event.stopPropagation();
                          }}
                          onKeyDown={(event) => {
                            if (event.key === "Enter") {
                              event.preventDefault();
                              saveRename(session.id, session.title);
                            } else if (event.key === "Escape") {
                              event.preventDefault();
                              cancelRename();
                            }
                          }}
                          autoFocus
                        />
                      ) : (
                        <>
                          <div className="session-title-row">
                            <span className="session-title-text">{displayTitle}</span>
                            {isGenerating ? (
                              <span className="session-live-pill">
                                <span className="session-live-dot" />
                                {t("chat.generating")}
                              </span>
                            ) : null}
                          </div>
                          <span className="session-meta" title={session.updatedAt}>
                            {formatSessionActivityTime(session.updatedAt)}
                          </span>
                        </>
                      )}
                    </button>
                    <div className="session-actions">
                      {isRenaming ? (
                        <>
                          <button
                            type="button"
                            className="session-action-btn"
                            aria-label={saveSessionNameLabel}
                            title={saveSessionNameLabel}
                            onClick={() => {
                              saveRename(session.id, session.title);
                            }}
                          >
                            <Check size={14} />
                          </button>
                          <button
                            type="button"
                            className="session-action-btn"
                            aria-label={cancelRenameLabel}
                            title={cancelRenameLabel}
                            onClick={cancelRename}
                          >
                            <X size={14} />
                          </button>
                        </>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="session-action-btn"
                            aria-label={`${renameSessionLabel} ${displayTitle}`}
                            title={renameSessionLabel}
                            onClick={() => {
                              beginRename(session.id, session.title);
                            }}
                          >
                            <PencilLine size={14} />
                          </button>
                          <button
                            type="button"
                            className="session-delete"
                            aria-label={`${t("chat.deleteSession")} ${displayTitle}`}
                            onClick={() => {
                              removeSessionTitleOverride(session.id);
                              void deleteSession(session.id);
                            }}
                            disabled={deletingSessionId === session.id}
                          >
                            <Trash2 size={14} />
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                );
              })
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
                  {selectedSessionDisplayTitle ?? t("chat.untitledSession")}
                </div>
              </div>
              <div className="chat-topline-meta">
                <div className="chat-topline-status-row">
                  <span title={selectedSession?.updatedAt ?? ""}>
                    {selectedSession
                      ? formatSessionActivityTime(selectedSession.updatedAt)
                      : t("chat.noHistory")}
                  </span>
                  <span
                    className="chat-status-dot"
                    title={t("status.connected")}
                    aria-label={t("status.connected")}
                  />
                </div>
                <span
                  className="chat-topline-model"
                  title={currentModel || t("chat.currentModel.pending")}
                >
                  {currentModel || t("chat.currentModel.pending")}
                </span>
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

            <div className="chat-composer-dock">
              {showMascot ? (
                <ChatMascot
                  isChinese={isChinese}
                  streamPhase={streamPhase}
                  activeToolCount={activeTools.length}
                  isSubmitting={isSubmitting}
                />
              ) : null}

              <form
                className="composer composer-inline"
                onSubmit={(event) => {
                  event.preventDefault();
                  void handleSubmit();
                }}
              >
                <div className="composer-shell" style={{ alignItems: "flex-end" }}>
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
                    disabled={isSubmitting || !composerText.trim() || !canAccessProtectedApi}
                  >
                    <SendHorizontal size={16} />
                    <span className="sr-only">
                      {isSubmitting ? "Sending..." : t("chat.send")}
                    </span>
                  </button>
                </div>
              </form>
            </div>
          </div>
        </Panel>

        <Panel eyebrow={t("chat.panels.inspector")} title={t("status.local")}>
          <div className="chat-inspector-body metric-grid-scroll">
            <div className="chat-inspector-summary">
              <div className="chat-inspector-summary-row">
                <span className="chat-inspector-summary-label">{t("status.providerReady")}</span>
                <strong className="chat-inspector-summary-value">
                  {selectedSession
                    ? t("chat.inspector.sessionLoaded")
                    : t("chat.inspector.waitingForSession")}
                </strong>
              </div>
              <div className="chat-inspector-summary-row">
                <span className="chat-inspector-summary-label">{t("status.memoryHealthy")}</span>
                <strong className="chat-inspector-summary-value">
                  {messages.length > 0 ? `${messages.length} messages` : "No history"}
                </strong>
              </div>
              <div className="chat-inspector-summary-row">
                <span className="chat-inspector-summary-label">{t("chat.memoryWindow.label")}</span>
                <strong className="chat-inspector-summary-value">
                  {memoryWindow !== null
                    ? t("chat.memoryWindow.value", { count: memoryWindow })
                    : t("chat.memoryWindow.pending")}
                </strong>
              </div>
            </div>

            <div className="chat-inspector-section">
              <div className="metric-label">{t("chat.inspector.recentTools")}</div>
              {recentTools.length > 0 ? (
                <div className="chat-inspector-tool-list">
                  {recentTools.map((tool) => (
                    <div key={`${tool.toolId}-${tool.finishedAt}`} className="chat-inspector-tool-item">
                      <div className="chat-inspector-tool-head">
                        <strong className="chat-inspector-tool-name">{tool.label}</strong>
                        <div className="chat-inspector-tool-side">
                          <span>{formatRecentToolTime(tool.finishedAt)}</span>
                          <span
                            className={`chat-recent-tool-dot chat-recent-tool-dot-${resolveRecentToolStatusTone(tool)}`}
                            aria-label={t(`chat.toolStatus.${tool.status}`)}
                            title={t(`chat.toolStatus.${tool.status}`)}
                          />
                        </div>
                      </div>
                      {resolveRecentToolDetail(tool) ? (
                        <div className="chat-inspector-tool-meta">
                          <span>{resolveRecentToolDetail(tool)}</span>
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : (
                <div className="chat-inspector-empty">{t("chat.inspector.noRecentTools")}</div>
              )}
            </div>
          </div>
        </Panel>
      </div>
    </div>
  );
}
