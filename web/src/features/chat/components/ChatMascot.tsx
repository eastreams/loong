import { useEffect, useMemo, useRef, useState } from "react";
import "./chat-mascot.css";
import type { StreamPhase } from "../hooks/useChatSessions";

interface ChatMascotProps {
  isChinese: boolean;
  streamPhase: StreamPhase;
  activeToolCount: number;
  isSubmitting: boolean;
}

type MascotTone = "outline" | "face" | "ear" | "eye" | "blush" | "feature";

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

const MOOD_MENU_ITEMS: { mood: MascotMood; labelZh: string; labelEn: string; icon: string }[] = [
  { mood: "idle",     labelZh: "待机",   labelEn: "Idle",     icon: "😺" },
  { mood: "thinking", labelZh: "思考中", labelEn: "Thinking", icon: "🤔" },
  { mood: "success",  labelZh: "开心",   labelEn: "Success",  icon: "😸" },
  { mood: "error",    labelZh: "出错",   labelEn: "Error",    icon: "😵" },
];

type MascotMood = "idle" | "thinking" | "error" | "success";

// ── Pixel art frames ──────────────────────────────────────────────

// Default idle — open eyes, neutral mouth
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

// Thinking — half-closed eyes looking up, small mouth
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

// Error — X_X dead eyes, flat mouth, no blush
const ERROR_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offfmfmffmfmfffo  ",
  " offffffmffmffffffo ",
  " offfffffffffffffo  ",
  " offffffmmmfffffffo ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

// Success — happy ^_^ arc eyes, big smile, extra blush
const SUCCESS_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offffmfffmffffo   ",
  " offfmfmffmfmffffo  ",
  " offffbbffffbbffffo ",
  " offffffmmmfffffffo ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

const BUBBLES_ZH = ["喵呜", "收到啦", "看着呢", "继续吧", "好耶"];
const BUBBLES_EN = ["mew.", "noted.", "watching.", "keep going.", "yay."];

// Moved to top for MOOD_MENU_ITEMS reference
// type MascotMood = "idle" | "thinking" | "error" | "success";

function resolveMood(
  streamPhase: StreamPhase,
  activeToolCount: number,
  isSubmitting: boolean,
  hasSuccessHold: boolean,
): MascotMood {
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

type MascotChatState = "idle" | "connecting" | "thinking" | "streaming" | "tool";

function resolveChatState(
  streamPhase: StreamPhase,
  activeToolCount: number,
  isSubmitting: boolean,
): MascotChatState {
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
): string | null {
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
      if (!tone) {
        return;
      }

      cells.push({ x, y, tone });
    });
  });

  return {
    width,
    height: rows.length,
    cells,
  };
}

// ── Thinking dots overlay ─────────────────────────────────────────

function ThinkingDots() {
  return (
    <div className="chat-mascot-thinking-dots" aria-hidden="true">
      <span className="chat-mascot-dot chat-mascot-dot-1" />
      <span className="chat-mascot-dot chat-mascot-dot-2" />
      <span className="chat-mascot-dot chat-mascot-dot-3" />
    </div>
  );
}

// ── Frame renderer ────────────────────────────────────────────────

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

// ── Component ─────────────────────────────────────────────────────

export function ChatMascot({
  isChinese,
  streamPhase,
  activeToolCount,
  isSubmitting,
}: ChatMascotProps) {
  const [isActing, setIsActing] = useState(false);
  const [isCoolingDown, setIsCoolingDown] = useState(false);
  const [jumpFrameIndex, setJumpFrameIndex] = useState(0);
  const [bubbleText, setBubbleText] = useState<string | null>(null);
  const [isStatusBubbleVisible, setIsStatusBubbleVisible] = useState(false);
  const [hasSuccessHold, setHasSuccessHold] = useState(false);
  const [previewMood, setPreviewMood] = useState<MascotMood | null>(null);
  const [contextMenuPos, setContextMenuPos] = useState<{ x: number; y: number } | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const prevStreamPhaseRef = useRef<StreamPhase>(streamPhase);
  const bubblePool = isChinese ? BUBBLES_ZH : BUBBLES_EN;

  const chatState = resolveChatState(streamPhase, activeToolCount, isSubmitting);
  const naturalMood = resolveMood(streamPhase, activeToolCount, isSubmitting, hasSuccessHold);
  const mood = previewMood ?? naturalMood;
  const statusBubbleText = resolveStatusBubble(chatState, isChinese, activeToolCount);
  const bubbleContent = bubbleText ?? statusBubbleText;
  const displayBubbleText = isStatusBubbleVisible ? bubbleContent : null;

  const idleFrame = useMemo(() => buildFrame(IDLE_ROWS), []);
  const thinkingFrame = useMemo(() => buildFrame(THINKING_ROWS), []);
  const errorFrame = useMemo(() => buildFrame(ERROR_ROWS), []);
  const successFrame = useMemo(() => buildFrame(SUCCESS_ROWS), []);

  // Select frame based on mood
  const frameMap: Record<MascotMood, PixelFrame> = {
    idle: idleFrame,
    thinking: thinkingFrame,
    error: errorFrame,
    success: successFrame,
  };
  const currentFrame = frameMap[mood];

  // Auto-clear preview mood
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

  // Close context menu on outside click / escape
  useEffect(() => {
    if (!contextMenuPos) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) {
        setContextMenuPos(null);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setContextMenuPos(null);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [contextMenuPos]);

  // Detect successful completion: streaming/thinking → idle
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

  // Auto-clear success hold after a delay
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

  // Jump animation on click
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

  // Click cooldown
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

  // Status bubble visibility
  useEffect(() => {
    if (!bubbleContent) {
      setIsStatusBubbleVisible(false);
      return;
    }

    setIsStatusBubbleVisible(true);
    const statusBubbleTimer = window.setTimeout(() => {
      setIsStatusBubbleVisible(false);
    }, BUBBLE_VISIBLE_MS);

    return () => {
      window.clearTimeout(statusBubbleTimer);
    };
  }, [bubbleContent]);

  // Manual bubble auto-dismiss
  useEffect(() => {
    if (!bubbleText) {
      return;
    }

    const manualBubbleTimer = window.setTimeout(() => {
      setBubbleText(null);
    }, BUBBLE_VISIBLE_MS);

    return () => {
      window.clearTimeout(manualBubbleTimer);
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
    if (isCoolingDown) {
      return;
    }

    setIsActing(true);
    setIsCoolingDown(true);
    setBubbleText(randomBubble());
  }

  function handleContextMenu(event: React.MouseEvent) {
    event.preventDefault();
    const menuWidth = 160;
    const menuHeight = 180;
    const pad = 8;

    // Place to the right of cursor, but above it
    let x = event.clientX + 4;
    let y = event.clientY - menuHeight - 4;

    // Clamp within viewport
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
  }

  function handleMenuSelect(selectedMood: MascotMood) {
    setPreviewMood(selectedMood);
    setContextMenuPos(null);
  }

  return (
    <>
      <button
        type="button"
        className={`chat-mascot chat-mascot-${chatState} chat-mascot-mood-${mood}${isActing ? " is-acting" : ""}${previewMood ? " is-previewing" : ""}`}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        disabled={isCoolingDown}
        aria-label={isChinese ? "点击 Qoong" : "Tap Qoong"}
        title={isChinese ? "点一下 Qoong" : "Tap Qoong"}
      >
        {mood === "thinking" ? <ThinkingDots /> : null}

        {displayBubbleText ? (
          <div className="chat-mascot-bubble">
            <span className="chat-mascot-bubble-icon">🐾</span>
            <span className="chat-mascot-bubble-text">{displayBubbleText}</span>
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
          <div className="chat-mascot-menu-title">
            {isChinese ? "表情预览" : "Preview Mood"}
          </div>
          {MOOD_MENU_ITEMS.map((item) => (
            <button
              key={item.mood}
              type="button"
              className={`chat-mascot-menu-item${mood === item.mood ? " is-active" : ""}`}
              onClick={() => handleMenuSelect(item.mood)}
            >
              <span className="chat-mascot-menu-item-icon">{item.icon}</span>
              <span>{isChinese ? item.labelZh : item.labelEn}</span>
            </button>
          ))}
        </div>
      ) : null}
    </>
  );
}
