import { Plus, SendHorizontal, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Panel } from "../../../components/surfaces/Panel";
import { chatApi, type ChatMessage, type ChatSessionSummary } from "../api";

export default function ChatPage() {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [composerText, setComposerText] = useState("");
  const [isLoadingSessions, setIsLoadingSessions] = useState(true);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [deletingSessionId, setDeletingSessionId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

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
          setError(loadError instanceof Error ? loadError.message : "Failed to load sessions");
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
  }, []);

  useEffect(() => {
    if (!selectedSessionId) {
      setMessages([]);
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
        }
      } catch (loadError) {
        if (!cancelled) {
          setError(loadError instanceof Error ? loadError.message : "Failed to load history");
          setMessages([]);
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
  }, [selectedSessionId]);

  async function refreshSessions(preferredSessionId?: string) {
    const loadedSessions = await chatApi.listSessions();
    setSessions(loadedSessions);
    setSelectedSessionId(
      (current) => preferredSessionId ?? current ?? loadedSessions[0]?.id ?? null,
    );
  }

  async function handleSubmit() {
    const input = composerText.trim();
    if (!input || isSubmitting) {
      return;
    }

    const optimisticUserMessage: ChatMessage = {
      id: `local-user-${Date.now()}`,
      role: "user",
      content: input,
      createdAt: new Date().toISOString(),
    };
    const previousMessages = messages;

    setError(null);
    setIsSubmitting(true);
    setMessages((current) => [...current, optimisticUserMessage]);
    setComposerText("");

    try {
      const sessionId =
        selectedSessionId ?? (await chatApi.createSession(input.slice(0, 48)));

      const assistantMessage = await chatApi.submitTurn(sessionId, input);
      setMessages((current) => [...current, assistantMessage]);
      await refreshSessions(sessionId);
    } catch (submitError) {
      setMessages(previousMessages);
      setError(submitError instanceof Error ? submitError.message : "Failed to send message");
      setComposerText(input);
    } finally {
      setIsSubmitting(false);
    }
  }

  async function handleDeleteSession(sessionId: string) {
    if (deletingSessionId) {
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
        }
      }
    } catch (deleteError) {
      setError(deleteError instanceof Error ? deleteError.message : "Failed to delete session");
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
                setError(null);
              }}
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

        <Panel
          eyebrow={t("status.local")}
          title={t("chat.title")}
          aside={<div className="chat-main-status">{t("status.connected")}</div>}
        >
          <div className="chat-main">
            <div className="chat-topline">
              <div>
                <div className="panel-eyebrow">{t("chat.eyebrow")}</div>
                <div className="chat-session-title">
                  {selectedSession?.title ?? "Untitled session"}
                </div>
              </div>
              <div className="chat-topline-meta">
                {selectedSession?.updatedAt ?? "No history"}
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
                    <p>{message.content}</p>
                  </article>
                ))}
            </div>

            <form
              className="composer composer-inline"
              onSubmit={(event) => {
                event.preventDefault();
                void handleSubmit();
              }}
            >
              <textarea
                className="composer-input"
                rows={3}
                placeholder={t("chat.inputPlaceholder")}
                value={composerText}
                onChange={(event) => {
                  setComposerText(event.target.value);
                }}
                disabled={isSubmitting}
              />
              <div className="composer-actions">
                {deletingSessionId ? (
                  <div className="composer-hint">{t("chat.deleting")}</div>
                ) : null}
                <button
                  type="submit"
                  className="hero-btn hero-btn-primary"
                  disabled={isSubmitting || !composerText.trim()}
                >
                  <SendHorizontal size={16} />
                  <span>{isSubmitting ? "Sending..." : t("chat.send")}</span>
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
          </div>
        </Panel>
      </div>
    </div>
  );
}
