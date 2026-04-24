import { useCallback, useRef } from "react";
import { ApiRequestError } from "../../../lib/api/client";
import {
  chatApi,
  isInternalAssistantRecordContent,
  stripInternalAssistantRecordPrefix,
  type ChatTurnStreamEvent,
} from "../api";
import {
  CHAT_MASCOT_PROFILE,
  CHAT_MASCOT_SESSION_ID,
  getMascotSystemPrompt,
} from "../mascotProfile";

interface UseMascotAgentParams {
  isChinese: boolean;
  canAccessProtectedApi: boolean;
  markUnauthorized: () => void;
}

interface MascotReplyOptions {
  pageContext?: string;
  signal?: AbortSignal;
}

interface MascotReplyResult {
  sessionId: string;
  turnId: string;
  content: string;
}

function buildMascotInput(
  input: string,
  isChinese: boolean,
  pageContext?: string,
): string {
  const systemPrompt = getMascotSystemPrompt(isChinese);
  const outputRules = isChinese
    ? [
        "回复要求：",
        "- 默认简短，除非明确要求详细说明",
        "- 保持 Qoong 的角色感，但不要抢主 agent 的职责",
        "- 如果提供了页面上下文，只在确实相关时引用",
      ].join("\n")
    : [
        "Reply requirements:",
        "- Stay brief by default unless detail is explicitly needed",
        "- Keep Qoong's personality, but do not replace the main agent",
        "- Use page context only when it is actually relevant",
      ].join("\n");
  const pageContextRules = pageContext?.trim()
    ? isChinese
      ? [
          "页面反馈规则：",
          "- 优先回应当前会话内容本身，而不是总结页面运行状态",
          "- 可以指出对话推进到哪、哪里可能卡住、下一步可以留意什么",
          "- 不要罗列 URL、控件、按钮或页面结构",
          "- 只有真的看到错误、阻塞或待处理事项时才提状态",
        ].join("\n")
      : [
          "Page feedback rules:",
          "- Respond to the visible conversation itself instead of summarizing runtime state",
          "- Mention where the discussion is, what may be stuck, or what to watch next",
          "- Do not list URLs, controls, buttons, or page structure",
          "- Only mention status when an error, blocker, or pending item is actually visible",
        ].join("\n")
    : null;

  const sections = [
    systemPrompt,
    pageContext?.trim()
      ? isChinese
        ? `页面上下文：\n${pageContext.trim()}`
        : `Page context:\n${pageContext.trim()}`
      : null,
    isChinese ? `当前任务：\n${input.trim()}` : `Current task:\n${input.trim()}`,
    outputRules,
    pageContextRules,
  ].filter(Boolean);

  return sections.join("\n\n");
}

function isAssistantDelta(
  event: ChatTurnStreamEvent,
): event is Extract<ChatTurnStreamEvent, { type: "message.delta" }> {
  return event.type === "message.delta" && event.role === "assistant";
}

export function useMascotAgent({
  isChinese,
  canAccessProtectedApi,
  markUnauthorized,
}: UseMascotAgentParams) {
  const ensureSessionPromiseRef = useRef<Promise<string | null> | null>(null);

  const ensureSession = useCallback(async (): Promise<string | null> => {
    if (!canAccessProtectedApi) {
      return null;
    }

    if (ensureSessionPromiseRef.current) {
      return ensureSessionPromiseRef.current;
    }

    const promise = Promise.resolve(CHAT_MASCOT_SESSION_ID);
    ensureSessionPromiseRef.current = promise;

    try {
      return await promise;
    } finally {
      ensureSessionPromiseRef.current = null;
    }
  }, [canAccessProtectedApi]);

  const requestReply = useCallback(
    async (
      input: string,
      options?: MascotReplyOptions,
    ): Promise<MascotReplyResult | null> => {
      const trimmedInput = input.trim();
      if (!trimmedInput || !canAccessProtectedApi) {
        return null;
      }

      const sessionId = await ensureSession();
      if (!sessionId) {
        return null;
      }

      try {
        const history = await chatApi.loadHistory(sessionId);
        if (history.length >= CHAT_MASCOT_PROFILE.contextMessageLimit) {
          await chatApi.deleteSession(sessionId).catch(() => undefined);
        }
      } catch (error) {
        if (error instanceof ApiRequestError && error.status === 404) {
          // Fixed mascot session ids are valid even before the first persisted turn.
        } else if (error instanceof ApiRequestError && error.status === 401) {
          markUnauthorized();
          return null;
        } else {
          throw error;
        }
      }

      const turn = await chatApi.createTurn(
        sessionId,
        buildMascotInput(trimmedInput, isChinese, options?.pageContext),
      );

      let bufferedInternalContent = "";
      let assistantContent = "";

      await chatApi.streamTurn(
        sessionId,
        turn.turnId,
        {
          onEvent: (event) => {
            if (isAssistantDelta(event)) {
              const nextContent = `${bufferedInternalContent}${assistantContent}${event.delta}`;
              if (isInternalAssistantRecordContent(nextContent)) {
                const visibleContent = stripInternalAssistantRecordPrefix(nextContent);
                if (visibleContent) {
                  bufferedInternalContent = "";
                  assistantContent = visibleContent;
                } else {
                  bufferedInternalContent = nextContent;
                }
                return;
              }

              assistantContent = `${assistantContent}${event.delta}`;
              return;
            }

            if (event.type === "turn.completed" && event.message.role === "assistant") {
              assistantContent = stripInternalAssistantRecordPrefix(event.message.content);
              return;
            }

            if (event.type === "turn.failed") {
              throw new Error(event.message);
            }
          },
        },
        {
          signal: options?.signal,
        },
      );

      return {
        sessionId,
        turnId: turn.turnId,
        content: assistantContent.trim(),
      };
    },
    [canAccessProtectedApi, ensureSession, isChinese, markUnauthorized],
  );

  return {
    primeSession: ensureSession,
    requestReply,
  };
}
