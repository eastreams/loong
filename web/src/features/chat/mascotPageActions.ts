import type { Theme } from "../../contexts/ThemeContextValue";
import {
  chatApi,
  type MascotBrowserSearchResponse,
  type MascotBrowserThemeToggleResponse,
} from "./api";

export type MascotPageActionId = "toggle-theme";

export interface MascotPageActionResult {
  actionId: MascotPageActionId;
  targetFound: boolean;
  performed: boolean;
  changed: boolean;
  nextTheme: Theme | null;
}

export async function performMascotBrowserThemeToggle(
  pageUrl: string,
): Promise<MascotBrowserThemeToggleResponse> {
  return chatApi.mascotToggleThemeWithBrowserCompanion(pageUrl);
}

export async function performMascotBrowserSearch(
  query?: string,
): Promise<MascotBrowserSearchResponse> {
  return chatApi.mascotSearchWithBrowserCompanion(query);
}

function readDocumentTheme(): Theme | null {
  if (typeof document === "undefined") {
    return null;
  }

  const theme = document.documentElement.getAttribute("data-theme");
  return theme === "dark" || theme === "light" ? theme : null;
}

function findThemeToggleButton(): HTMLButtonElement | null {
  if (typeof document === "undefined") {
    return null;
  }

  const directMatch = document.querySelector<HTMLButtonElement>(
    '[data-mascot-action="toggle-theme"]',
  );
  if (directMatch) {
    return directMatch;
  }

  const buttons = Array.from(document.querySelectorAll<HTMLButtonElement>("button"));
  return (
    buttons.find((button) => {
      const label = `${button.getAttribute("aria-label") ?? ""} ${button.getAttribute("title") ?? ""}`
        .trim()
        .toLowerCase();
      return label.includes("theme") || label.includes("主题");
    }) ?? null
  );
}

function waitForAnimation(animation: Animation): Promise<void> {
  return animation.finished.then(
    () => undefined,
    () => undefined,
  );
}

function elementCenter(rect: DOMRect): { x: number; y: number } {
  return {
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

async function animateMascotVirtualCursorClick(target: HTMLElement): Promise<void> {
  if (typeof document === "undefined" || typeof window === "undefined") {
    return;
  }

  if (typeof target.scrollIntoView === "function") {
    target.scrollIntoView({
      block: "nearest",
      inline: "nearest",
      behavior: "smooth",
    });
    await new Promise((resolve) => window.setTimeout(resolve, 260));
  }

  const targetRect = target.getBoundingClientRect();
  const mascotRect = document.querySelector<HTMLElement>(".chat-mascot")?.getBoundingClientRect();
  const start = mascotRect
    ? elementCenter(mascotRect)
    : { x: window.innerWidth * 0.5, y: window.innerHeight * 0.82 };
  const end = elementCenter(targetRect);
  const lift = Math.min(28, Math.max(8, Math.abs(end.y - start.y) * 0.08));
  const mid = {
    x: start.x + (end.x - start.x) * 0.58,
    y: start.y + (end.y - start.y) * 0.58 - lift,
  };
  const early = {
    x: start.x + (end.x - start.x) * 0.26,
    y: start.y + (end.y - start.y) * 0.26 - lift * 0.55,
  };
  const late = {
    x: start.x + (end.x - start.x) * 0.82,
    y: start.y + (end.y - start.y) * 0.82 - lift * 0.35,
  };

  const cursor = document.createElement("div");
  cursor.className = "chat-mascot-virtual-cursor";
  cursor.setAttribute("aria-hidden", "true");
  cursor.innerHTML = `
    <svg viewBox="0 0 8 10" class="chat-mascot-virtual-cursor-art" focusable="false">
      <path class="chat-mascot-virtual-cursor-shadow" d="M1 1h1v1h1v1h1v1h1v1h1v1H5v1h1v1H5v1H4V8H3v1H2V8H1V1z" />
      <path class="chat-mascot-virtual-cursor-fill" d="M0 0h1v1h1v1h1v1h1v1h1v1H4v1h1v1H4v1H3V7H2v1H1V7H0V0z" />
      <path class="chat-mascot-virtual-cursor-edge" d="M0 0h1v1H0V0zm1 1h1v1H1V1zm1 1h1v1H2V2zm1 1h1v1H3V3zm1 1h1v1H4V4zM0 1h1v6H0V1zm1 6h1v1H1V7zm1 1h1v1H2V8zm1-1h1v1H3V7zm1-1h1v1H4V6z" />
      <rect class="chat-mascot-virtual-cursor-spark" x="5" y="0" width="1" height="1" />
      <rect class="chat-mascot-virtual-cursor-spark" x="7" y="2" width="1" height="1" />
    </svg>
  `;
  document.body.append(cursor);

  const transform = (point: { x: number; y: number }, scale = 1) =>
    `translate(${Math.round(point.x)}px, ${Math.round(point.y)}px) scale(${scale})`;

  try {
    const move = cursor.animate(
      [
        { transform: transform(start, 0.78), opacity: 0 },
        { transform: transform(start, 0.96), opacity: 1, offset: 0.1 },
        { transform: transform(early, 1), opacity: 1, offset: 0.34 },
        { transform: transform(mid, 1.03), opacity: 1, offset: 0.58 },
        { transform: transform(late, 1), opacity: 1, offset: 0.82 },
        { transform: transform({ x: end.x - 4, y: end.y - 4 }, 0.98), opacity: 1, offset: 0.94 },
        { transform: transform(end, 0.96), opacity: 1 },
      ],
      {
        duration: 1680,
        easing: "cubic-bezier(0.16, 0.84, 0.22, 1)",
        fill: "forwards",
      },
    );
    await waitForAnimation(move);

    await new Promise((resolve) => window.setTimeout(resolve, 320));

    cursor.classList.add("is-clicking");
    const click = cursor.animate(
      [
        { transform: transform(end, 0.96) },
        { transform: transform({ x: end.x + 1, y: end.y + 1 }, 0.84), offset: 0.45 },
        { transform: transform(end, 0.98) },
      ],
      {
        duration: 340,
        easing: "cubic-bezier(0.32, 0, 0.2, 1)",
        fill: "forwards",
      },
    );
    await waitForAnimation(click);

    await new Promise((resolve) => window.setTimeout(resolve, 220));
  } finally {
    cursor.remove();
  }
}

function waitForThemeChange(previousTheme: Theme | null, timeoutMs = 1200): Promise<Theme | null> {
  if (typeof MutationObserver === "undefined" || typeof document === "undefined") {
    return Promise.resolve(readDocumentTheme());
  }

  return new Promise((resolve) => {
    let settled = false;

    const finish = (theme: Theme | null) => {
      if (settled) {
        return;
      }
      settled = true;
      observer.disconnect();
      window.clearTimeout(timeoutId);
      resolve(theme);
    };

    const observer = new MutationObserver(() => {
      const nextTheme = readDocumentTheme();
      if (nextTheme && nextTheme !== previousTheme) {
        finish(nextTheme);
      }
    });

    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });

    const timeoutId = window.setTimeout(() => {
      finish(readDocumentTheme());
    }, timeoutMs);
  });
}

async function performThemeToggle(): Promise<MascotPageActionResult> {
  const target = findThemeToggleButton();
  const previousTheme = readDocumentTheme();

  if (!target || target.disabled) {
    return {
      actionId: "toggle-theme",
      targetFound: false,
      performed: false,
      changed: false,
      nextTheme: previousTheme,
    };
  }

  await animateMascotVirtualCursorClick(target);
  target.click();
  const nextTheme = await waitForThemeChange(previousTheme);

  return {
    actionId: "toggle-theme",
    targetFound: true,
    performed: true,
    changed: nextTheme !== null && nextTheme !== previousTheme,
    nextTheme,
  };
}

export async function performMascotPageAction(
  actionId: MascotPageActionId,
): Promise<MascotPageActionResult> {
  switch (actionId) {
    case "toggle-theme":
      return performThemeToggle();
    default:
      return {
        actionId,
        targetFound: false,
        performed: false,
        changed: false,
        nextTheme: readDocumentTheme(),
      };
  }
}

export function getMascotThemeActionLabel(theme: Theme, isChinese: boolean): string {
  if (isChinese) {
    return theme === "dark" ? "开下灯" : "关下灯";
  }

  return theme === "dark" ? "Turn the lights on" : "Dim the lights";
}

export function getMascotThemeActionTooltip(theme: Theme, isChinese: boolean): string {
  if (isChinese) {
    return theme === "dark"
      ? "让 Qoong 帮你切回亮色模式。"
      : "让 Qoong 帮你切到深色模式。";
  }

  return theme === "dark"
    ? "Let Qoong switch the page back to light mode."
    : "Let Qoong switch the page into dark mode.";
}
