import { useEffect, useMemo, useState } from "react";
import "./chat-mascot.css";

interface ChatMascotProps {
  isChinese: boolean;
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

const ACTION_ROWS = [
  "      ee   ee      ",
  "     eeee eeee     ",
  "    eeffffffffee    ",
  "   eoffffffffffoe   ",
  "  offfzzzz yyyyfffo ",
  " offffzzzzyyyyffffo ",
  " offffbfffmmfbffffo ",
  " offfffffmmmmfffffo ",
  "  offffffffffffffo  ",
  "   offffffffffffo   ",
  "    ooffffffoo    ",
  "      o  oo  o      ",
];

const BUBBLES_ZH = ["喵呜", "收到啦", "看着呢", "继续吧", "好耶"];
const BUBBLES_EN = ["mew.", "noted.", "watching.", "keep going.", "yay."];

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

function renderFrame(frame: PixelFrame) {
  return (
    <svg
      className="chat-mascot-art"
      viewBox={`0 0 ${frame.width} ${frame.height}`}
      aria-hidden="true"
      preserveAspectRatio="xMidYMax meet"
    >
      {frame.cells.map((cell) => (
        <rect
          key={`${cell.x}-${cell.y}-${cell.tone}`}
          x={cell.x}
          y={cell.y}
          width="1"
          height="1"
          className={`chat-mascot-pixel chat-mascot-pixel-${cell.tone}`}
        />
      ))}
    </svg>
  );
}

export function ChatMascot({ isChinese }: ChatMascotProps) {
  const [isActing, setIsActing] = useState(false);
  const [bubbleText, setBubbleText] = useState<string | null>(null);
  const bubblePool = isChinese ? BUBBLES_ZH : BUBBLES_EN;

  const idleFrame = useMemo(() => buildFrame(IDLE_ROWS), []);
  const actionFrame = useMemo(() => buildFrame(ACTION_ROWS), []);
  const currentFrame = isActing ? actionFrame : idleFrame;

  useEffect(() => {
    if (!isActing) {
      return;
    }

    const settleTimer = window.setTimeout(() => {
      setIsActing(false);
    }, 1100);

    const bubbleTimer = window.setTimeout(() => {
      setBubbleText(null);
    }, 1800);

    return () => {
      window.clearTimeout(settleTimer);
      window.clearTimeout(bubbleTimer);
    };
  }, [isActing]);

  function handleClick() {
    const nextBubble = bubblePool[Math.floor(Math.random() * bubblePool.length)];
    setBubbleText(nextBubble);
    setIsActing(true);
  }

  return (
    <button
      type="button"
      className={`chat-mascot${isActing ? " is-acting" : ""}`}
      onClick={handleClick}
      aria-label={isChinese ? "点击 Qoong" : "Tap Qoong"}
      title={isChinese ? "点一下 Qoong" : "Tap Qoong"}
    >
      {bubbleText ? (
        <div className="chat-mascot-bubble">
          <span className="chat-mascot-bubble-bracket">[::</span>
          <span>{bubbleText}</span>
          <span className="chat-mascot-bubble-bracket">::]</span>
        </div>
      ) : null}
      <div className="chat-mascot-stage">{renderFrame(currentFrame)}</div>
    </button>
  );
}
