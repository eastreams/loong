import { useEffect, useMemo, useRef, useState } from "react";
import { useTheme } from "../../../hooks/useTheme";
import "./chat-mascot.css";
import type { StreamPhase } from "../hooks/useChatSessions";
import {
  getDailyMascotBubblePool,
  getMascotLocalDayKey,
} from "../mascotProfile";
import {
  getMascotThemeActionLabel,
  getMascotThemeActionTooltip,
} from "../mascotPageActions";

export interface MascotActionReply {
  text: string;
  url?: string | null;
  linkLabel?: string;
}

interface ChatMascotProps {
  isChinese: boolean;
  streamPhase: StreamPhase;
  activeToolCount: number;
  isSubmitting: boolean;
  onWake?: () => void;
  onReadPage?: () => Promise<string | null>;
  onSearchQoong?: () => Promise<MascotActionReply | string | null>;
  onToggleTheme?: () => Promise<string | null>;
}

type MascotTone = "outline" | "face" | "ear" | "eye" | "blush" | "feature";
type MascotMood = "idle" | "thinking" | "error" | "success";
type MascotChatState = "idle" | "connecting" | "thinking" | "streaming" | "tool";
type HoveredAction = "read_page" | "search_qoong" | "toggle_theme";

interface PixelCell {
  x: number;
  y: number;
  tone: MascotTone;
}

interface PixelFrame {
  width: number;
  height: number;
  cells: PixelCell[];
}

const PIXEL_MAP: Record<string, MascotTone> = {
  o: "outline",
  f: "face",
  e: "ear",
  y: "eye",
  b: "blush",
  m: "feature",
  z: "feature",
};

const JUMP_HEADROOM = 10;
const JUMP_FOOTROOM = 2;
const JUMP_FRAME_MS = 90;
const JUMP_FRAME_OFFSETS = [0, 1, -1, -4, -5, -3, -1, 0];
const BUBBLE_VISIBLE_MS = 3000;
const SUCCESS_HOLD_MS = 2400;
const PREVIEW_HOLD_MS = 3000;

const MOOD_MENU_ITEMS: Array<{
  mood: MascotMood;
  labelZh: string;
  labelEn: string;
}> = [
  { mood: "idle", labelZh: "待机", labelEn: "Idle" },
  { mood: "thinking", labelZh: "思考中", labelEn: "Thinking" },
  { mood: "success", labelZh: "开心", labelEn: "Success" },
  { mood: "error", labelZh: "出错", labelEn: "Error" },
];

const IDLE_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offfyyyyyyyyfffo  ",
  " offffyyyyyyyyffffo ",
  " offffbffffffbffffo ",
  " offfffffmmfffffffo ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

const THINKING_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offffffffffffffo  ",
  " offffmmfffmmffffo  ",
  " offffbffffffbffffo ",
  " offfffffffffffffo  ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

const ERROR_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offfmfmffmfmfffo  ",
  " offfffmffffmfffffo ",
  " offffmfmffmfmffffo ",
  " offfffffffffffffo  ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

const SUCCESS_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offffffffffffffo  ",
  " offffmmffffmmffffo ",
  " offffbbffffbbffffo ",
  " offfffffffffffffo  ",
  "  offffffmfmfffffo  ",
  "   offffffmfffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

function resolveMood(
  streamPhase: StreamPhase,
  activeToolCount: number,
  isSubmitting: boolean,
  hasSuccessHold: boolean,
  isBusyWithPageAction: boolean,
): MascotMood {
  if (isBusyWithPageAction) {
    return "thinking";
  }

  if (hasSuccessHold) {
    return "success";
  }

  if (
    streamPhase === "connecting" ||
    streamPhase === "thinking" ||
    streamPhase === "streaming" ||
    activeToolCount > 0 ||
    isSubmitting
  ) {
    return "thinking";
  }

  return "idle";
}

function resolveChatState(
  streamPhase: StreamPhase,
  activeToolCount: number,
  isSubmitting: boolean,
  isBusyWithPageAction: boolean,
): MascotChatState {
  if (isBusyWithPageAction) {
    return "thinking";
  }

  if (activeToolCount > 0) {
    return "tool";
  }

  if (streamPhase !== "idle") {
    return streamPhase;
  }

  return isSubmitting ? "thinking" : "idle";
}

function resolveStatusBubble(
  chatState: MascotChatState,
  isChinese: boolean,
  activeToolCount: number,
  isBusyWithPageAction: boolean,
): string | null {
  if (isBusyWithPageAction) {
    return isChinese ? "我在动一下页面。" : "adjusting the page";
  }

  if (isChinese) {
    switch (chatState) {
      case "connecting":
        return "接通中";
      case "thinking":
        return "思考中";
      case "streaming":
        return "回复中";
      case "tool":
        return activeToolCount > 1 ? `工具 x${activeToolCount}` : "用工具";
      case "idle":
        return null;
    }
  }

  switch (chatState) {
    case "connecting":
      return "connecting";
    case "thinking":
      return "thinking";
    case "streaming":
      return "replying";
    case "tool":
      return activeToolCount > 1 ? `tools x${activeToolCount}` : "using tool";
    case "idle":
      return null;
  }
}

function buildFrame(rows: string[]): PixelFrame {
  const width = rows.reduce((max, row) => Math.max(max, row.length), 0);
  const cells: PixelCell[] = [];

  rows.forEach((row, y) => {
    Array.from(row).forEach((char, x) => {
      const tone = PIXEL_MAP[char];
      if (tone) {
        cells.push({ x, y, tone });
      }
    });
  });

  return { width, height: rows.length, cells };
}

function ThinkingDots() {
  return (
    <div className="chat-mascot-thinking-dots" aria-hidden="true">
      <span className="chat-mascot-dot chat-mascot-dot-1" />
      <span className="chat-mascot-dot chat-mascot-dot-2" />
      <span className="chat-mascot-dot chat-mascot-dot-3" />
    </div>
  );
}

function renderFrame(frame: PixelFrame, jumpOffset: number) {
  const viewportHeight = frame.height + JUMP_HEADROOM + JUMP_FOOTROOM;

  return (
    <svg
      className="chat-mascot-art"
      viewBox={`0 0 ${frame.width} ${viewportHeight}`}
      aria-hidden="true"
      preserveAspectRatio="xMidYMax meet"
    >
      {frame.cells.map((cell) => (
        <rect
          key={`${cell.x}-${cell.y}-${cell.tone}`}
          x={cell.x}
          y={cell.y + JUMP_HEADROOM + jumpOffset}
          width="1"
          height="1"
          className={`chat-mascot-pixel chat-mascot-pixel-${cell.tone}`}
        />
      ))}
    </svg>
  );
}

export function ChatMascot({
  isChinese,
  streamPhase,
  activeToolCount,
  isSubmitting,
  onWake,
  onReadPage,
  onSearchQoong,
  onToggleTheme,
}: ChatMascotProps) {
  const { theme } = useTheme();
  const [isActing, setIsActing] = useState(false);
  const [isCoolingDown, setIsCoolingDown] = useState(false);
  const [isReadingPage, setIsReadingPage] = useState(false);
  const [isSearchingQoong, setIsSearchingQoong] = useState(false);
  const [isTogglingTheme, setIsTogglingTheme] = useState(false);
  const [jumpFrameIndex, setJumpFrameIndex] = useState(0);
  const [bubbleText, setBubbleText] = useState<string | null>(null);
  const [bubbleLink, setBubbleLink] = useState<MascotActionReply | null>(null);
  const [isStatusBubbleVisible, setIsStatusBubbleVisible] = useState(false);
  const [hasSuccessHold, setHasSuccessHold] = useState(false);
  const [previewMood, setPreviewMood] = useState<MascotMood | null>(null);
  const [contextMenuPos, setContextMenuPos] = useState<{ x: number; y: number } | null>(null);
  const [hoveredAction, setHoveredAction] = useState<HoveredAction | null>(null);
  const [hoveredActionPos, setHoveredActionPos] = useState<{ x: number; y: number } | null>(null);
  const [dayKey, setDayKey] = useState(() => getMascotLocalDayKey());
  const menuRef = useRef<HTMLDivElement | null>(null);
  const prevStreamPhaseRef = useRef<StreamPhase>(streamPhase);

  const bubblePool = useMemo(
    () => getDailyMascotBubblePool(isChinese, dayKey),
    [dayKey, isChinese],
  );
  const isBusyWithPageAction = isReadingPage || isSearchingQoong || isTogglingTheme;
  const chatState = resolveChatState(
    streamPhase,
    activeToolCount,
    isSubmitting,
    isBusyWithPageAction,
  );
  const naturalMood = resolveMood(
    streamPhase,
    activeToolCount,
    isSubmitting,
    hasSuccessHold,
    isBusyWithPageAction,
  );
  const mood = previewMood ?? naturalMood;
  const statusBubbleText = resolveStatusBubble(
    chatState,
    isChinese,
    activeToolCount,
    isBusyWithPageAction,
  );
  const bubbleContent = bubbleText ?? statusBubbleText;
  const displayBubbleText = isStatusBubbleVisible ? bubbleContent : null;
  const displayBubbleLink = displayBubbleText === bubbleText ? bubbleLink : null;

  const idleFrame = useMemo(() => buildFrame(IDLE_ROWS), []);
  const thinkingFrame = useMemo(() => buildFrame(THINKING_ROWS), []);
  const errorFrame = useMemo(() => buildFrame(ERROR_ROWS), []);
  const successFrame = useMemo(() => buildFrame(SUCCESS_ROWS), []);
  const currentFrame =
    {
      idle: idleFrame,
      thinking: thinkingFrame,
      error: errorFrame,
      success: successFrame,
    }[mood];

  const themeActionLabel = getMascotThemeActionLabel(theme, isChinese);
  const themeActionTooltip = getMascotThemeActionTooltip(theme, isChinese);

  useEffect(() => {
    if (!previewMood) {
      return;
    }

    const timer = window.setTimeout(() => {
      setPreviewMood(null);
    }, PREVIEW_HOLD_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [previewMood]);

  useEffect(() => {
    const now = new Date();
    const nextLocalMidnight = new Date(now);
    nextLocalMidnight.setHours(24, 0, 0, 50);

    const timer = window.setTimeout(() => {
      setDayKey(getMascotLocalDayKey());
    }, Math.max(1000, nextLocalMidnight.getTime() - now.getTime()));

    return () => {
      window.clearTimeout(timer);
    };
  }, [dayKey]);

  useEffect(() => {
    if (!contextMenuPos) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) {
        setContextMenuPos(null);
        setHoveredAction(null);
        setHoveredActionPos(null);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setContextMenuPos(null);
        setHoveredAction(null);
        setHoveredActionPos(null);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [contextMenuPos]);

  useEffect(() => {
    const wasActive =
      prevStreamPhaseRef.current === "streaming" ||
      prevStreamPhaseRef.current === "thinking" ||
      prevStreamPhaseRef.current === "connecting";
    const isNowIdle = streamPhase === "idle";

    if (wasActive && isNowIdle) {
      setHasSuccessHold(true);
    }

    prevStreamPhaseRef.current = streamPhase;
  }, [streamPhase]);

  useEffect(() => {
    if (!hasSuccessHold) {
      return;
    }

    const timer = window.setTimeout(() => {
      setHasSuccessHold(false);
    }, SUCCESS_HOLD_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [hasSuccessHold]);

  useEffect(() => {
    if (!isActing) {
      return;
    }

    setJumpFrameIndex(0);

    const jumpTimer = window.setInterval(() => {
      setJumpFrameIndex((current) => {
        const next = current + 1;
        if (next >= JUMP_FRAME_OFFSETS.length) {
          window.clearInterval(jumpTimer);
          return JUMP_FRAME_OFFSETS.length - 1;
        }
        return next;
      });
    }, JUMP_FRAME_MS);

    const settleTimer = window.setTimeout(() => {
      setIsActing(false);
      setJumpFrameIndex(0);
    }, 1100);

    return () => {
      window.clearInterval(jumpTimer);
      window.clearTimeout(settleTimer);
    };
  }, [isActing]);

  useEffect(() => {
    if (!isCoolingDown) {
      return;
    }

    const cooldownTimer = window.setTimeout(() => {
      setIsCoolingDown(false);
    }, 1000);

    return () => {
      window.clearTimeout(cooldownTimer);
    };
  }, [isCoolingDown]);

  useEffect(() => {
    if (!bubbleContent) {
      setIsStatusBubbleVisible(false);
      return;
    }

    setIsStatusBubbleVisible(true);
    const timer = window.setTimeout(() => {
      setIsStatusBubbleVisible(false);
    }, BUBBLE_VISIBLE_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [bubbleContent]);

  useEffect(() => {
    if (!bubbleText) {
      setBubbleLink(null);
      return;
    }

    const timer = window.setTimeout(() => {
      setBubbleText(null);
    }, BUBBLE_VISIBLE_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [bubbleText]);

  function randomBubble() {
    const candidateBubbles = bubblePool.filter((item) => item !== bubbleText);
    return (
      candidateBubbles[Math.floor(Math.random() * candidateBubbles.length)] ??
      bubblePool[0] ??
      null
    );
  }

  function handleClick() {
    if (isCoolingDown || isBusyWithPageAction) {
      return;
    }

    onWake?.();
    setIsActing(true);
    setIsCoolingDown(true);
    setBubbleLink(null);
    setBubbleText(randomBubble());
  }

  function handleContextMenu(event: React.MouseEvent) {
    event.preventDefault();

    const actionCount =
      Number(Boolean(onToggleTheme)) +
      Number(Boolean(onReadPage)) +
      Number(Boolean(onSearchQoong));
    const menuWidth = 196;
    const menuHeight = 92 + MOOD_MENU_ITEMS.length * 40 + (actionCount > 0 ? 40 + actionCount * 38 : 0);
    const pad = 8;

    let x = event.clientX + 4;
    let y = event.clientY - menuHeight - 4;

    if (x + menuWidth + pad > window.innerWidth) {
      x = event.clientX - menuWidth - 4;
    }
    if (y < pad) {
      y = pad;
    }
    if (y + menuHeight + pad > window.innerHeight) {
      y = window.innerHeight - menuHeight - pad;
    }

    setContextMenuPos({ x, y });
    setHoveredAction(null);
    setHoveredActionPos(null);
  }

  function handleMenuSelect(selectedMood: MascotMood) {
    setPreviewMood(selectedMood);
    setContextMenuPos(null);
    setHoveredAction(null);
    setHoveredActionPos(null);
  }

  function closeMenuAndTooltip() {
    setContextMenuPos(null);
    setHoveredAction(null);
    setHoveredActionPos(null);
  }

  function bindTooltip(action: HoveredAction, event: React.MouseEvent) {
    setHoveredAction(action);
    setHoveredActionPos({
      x: event.clientX + 14,
      y: event.clientY + 12,
    });
  }

  async function handleReadPage() {
    if (!onReadPage || isBusyWithPageAction) {
      return;
    }

    onWake?.();
    closeMenuAndTooltip();
    setIsReadingPage(true);
    setBubbleLink(null);
    setBubbleText(isChinese ? "我去看一下。" : "let me check");

    try {
      const reply = await onReadPage();
      setPreviewMood("success");
      setHasSuccessHold(true);
      setBubbleText(
        reply?.trim() ||
          (isChinese ? "我先看完了，暂时没有新的结论。" : "nothing new to report."),
      );
    } catch {
      setPreviewMood("error");
      setBubbleText(
        isChinese ? "这次没看成，你再让我试一次。" : "that read failed. try me again.",
      );
    } finally {
      setIsReadingPage(false);
    }
  }

  async function handleSearchQoong() {
    if (!onSearchQoong || isBusyWithPageAction) {
      return;
    }

    onWake?.();
    closeMenuAndTooltip();
    setIsSearchingQoong(true);
    setBubbleLink(null);
    setBubbleText(isChinese ? "我去搜一下美食。" : "let me search food");

    try {
      const reply = await onSearchQoong();
      setPreviewMood("success");
      setHasSuccessHold(true);
      if (reply && typeof reply === "object") {
        setBubbleLink(reply.url ? reply : null);
        setBubbleText(
          reply.text.trim() ||
            (isChinese ? "我找到第一个结果了。" : "I found the first result."),
        );
      } else {
        setBubbleLink(null);
        setBubbleText(
          reply?.trim() ||
            (isChinese
              ? "我把搜索结果带回来了。"
              : "I brought back the search result."),
        );
      }
    } catch {
      setBubbleLink(null);
      setPreviewMood("error");
      setBubbleText(isChinese ? "这次搜索没跑通。" : "that search did not land this time.");
    } finally {
      setIsSearchingQoong(false);
    }
  }

  async function handleToggleTheme() {
    if (!onToggleTheme || isBusyWithPageAction) {
      return;
    }

    onWake?.();
    closeMenuAndTooltip();
    setIsTogglingTheme(true);
    setBubbleLink(null);
    setBubbleText(
      isChinese
        ? theme === "dark"
          ? "我去开灯。"
          : "我去关灯。"
        : theme === "dark"
          ? "turning the lights on"
          : "dimming the lights",
    );

    try {
      const reply = await onToggleTheme();
      setPreviewMood("success");
      setHasSuccessHold(true);
      setBubbleText(
        reply?.trim() ||
          (isChinese ? "灯已经切好了。" : "the lights are switched."),
      );
    } catch {
      setPreviewMood("error");
      setBubbleText(
        isChinese ? "这次没按动开关。" : "the switch did not move this time.",
      );
    } finally {
      setIsTogglingTheme(false);
    }
  }

  return (
    <>
      <button
        type="button"
        className={`chat-mascot chat-mascot-${chatState} chat-mascot-mood-${mood}${isActing ? " is-acting" : ""}${previewMood ? " is-previewing" : ""}`}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        disabled={isCoolingDown || isBusyWithPageAction}
        aria-label={isChinese ? "点一下 Qoong" : "Tap Qoong"}
        title={isChinese ? "点一下 Qoong" : "Tap Qoong"}
      >
        {mood === "thinking" ? <ThinkingDots /> : null}

        {displayBubbleText ? (
          <div className="chat-mascot-bubble">
            <span className="chat-mascot-bubble-text">{displayBubbleText}</span>
            {displayBubbleLink?.url ? (
              <a
                className="chat-mascot-bubble-link"
                href={displayBubbleLink.url}
                target="_blank"
                rel="noreferrer"
                onClick={(event) => {
                  event.stopPropagation();
                }}
              >
                {displayBubbleLink.linkLabel ?? displayBubbleLink.url}
              </a>
            ) : null}
          </div>
        ) : null}

        <div className="chat-mascot-stage">
          {renderFrame(currentFrame, JUMP_FRAME_OFFSETS[jumpFrameIndex] ?? 0)}
        </div>
      </button>

      {contextMenuPos ? (
        <div
          ref={menuRef}
          className="chat-mascot-menu"
          style={{ left: contextMenuPos.x, top: contextMenuPos.y }}
        >
          {onToggleTheme || onReadPage || onSearchQoong ? (
            <>
              <div className="chat-mascot-menu-title">{isChinese ? "动作" : "Actions"}</div>
              {onSearchQoong ? (
                <button
                  type="button"
                  className="chat-mascot-menu-item"
                  onClick={() => {
                    void handleSearchQoong();
                  }}
                  onMouseEnter={(event) => {
                    bindTooltip("search_qoong", event);
                  }}
                  onMouseMove={(event) => {
                    bindTooltip("search_qoong", event);
                  }}
                  onMouseLeave={() => {
                    setHoveredAction(null);
                    setHoveredActionPos(null);
                  }}
                  disabled={isBusyWithPageAction}
                >
                  <span>{isChinese ? "搜一下美食" : "Search food"}</span>
                </button>
              ) : null}
              {onToggleTheme ? (
                <button
                  type="button"
                  className="chat-mascot-menu-item"
                  onClick={() => {
                    void handleToggleTheme();
                  }}
                  onMouseEnter={(event) => {
                    bindTooltip("toggle_theme", event);
                  }}
                  onMouseMove={(event) => {
                    bindTooltip("toggle_theme", event);
                  }}
                  onMouseLeave={() => {
                    setHoveredAction(null);
                    setHoveredActionPos(null);
                  }}
                  disabled={isBusyWithPageAction}
                >
                  <span>{themeActionLabel}</span>
                </button>
              ) : null}
              {onReadPage ? (
                <button
                  type="button"
                  className="chat-mascot-menu-item"
                  onClick={() => {
                    void handleReadPage();
                  }}
                  onMouseEnter={(event) => {
                    bindTooltip("read_page", event);
                  }}
                  onMouseMove={(event) => {
                    bindTooltip("read_page", event);
                  }}
                  onMouseLeave={() => {
                    setHoveredAction(null);
                    setHoveredActionPos(null);
                  }}
                  disabled={isBusyWithPageAction}
                >
                  <span>{isChinese ? "读取当前页面" : "Read current page"}</span>
                </button>
              ) : null}
              <div className="chat-mascot-menu-divider" />
            </>
          ) : null}

          <div className="chat-mascot-menu-title">
            {isChinese ? "表情预览" : "Preview mood"}
          </div>
          {MOOD_MENU_ITEMS.map((item) => (
            <button
              key={item.mood}
              type="button"
              className={`chat-mascot-menu-item${mood === item.mood ? " is-active" : ""}`}
              onClick={() => handleMenuSelect(item.mood)}
            >
              <span>{isChinese ? item.labelZh : item.labelEn}</span>
            </button>
          ))}
        </div>
      ) : null}

      {hoveredAction && hoveredActionPos ? (
        <div
          className="chat-mascot-tooltip"
          style={{ left: hoveredActionPos.x, top: hoveredActionPos.y }}
        >
          {hoveredAction === "toggle_theme" ? (
            themeActionTooltip
          ) : hoveredAction === "search_qoong" ? (
            isChinese ? (
              "Qoong 会在受管浏览器里搜索美食，再把第一个网址带回来。"
            ) : (
              "Qoong searches food in the managed browser and returns the first URL."
            )
          ) : isChinese ? (
            "Qoong 会读取当前页面可见内容，再给你一个简短反馈。"
          ) : (
            "Qoong reads the visible page content and replies briefly."
          )}
        </div>
      ) : null}
    </>
  );
}
