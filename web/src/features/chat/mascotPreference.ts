export const CHAT_MASCOT_ENABLED_STORAGE_KEY = "loongclaw.web.chat.mascotEnabled";
export const CHAT_MASCOT_TOGGLED_EVENT = "loongclaw:chat-mascot-toggled";

export function readChatMascotEnabled(): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  try {
    return window.localStorage.getItem(CHAT_MASCOT_ENABLED_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeChatMascotEnabled(enabled: boolean) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(CHAT_MASCOT_ENABLED_STORAGE_KEY, enabled ? "true" : "false");
    window.dispatchEvent(
      new CustomEvent(CHAT_MASCOT_TOGGLED_EVENT, {
        detail: { enabled },
      }),
    );
  } catch {
    // Ignore localStorage failures and keep the toggle best-effort.
  }
}
