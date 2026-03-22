const STORAGE_KEY = "loongclaw-web-token";

export function getStoredToken(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

export function setStoredToken(token: string) {
  try {
    localStorage.setItem(STORAGE_KEY, token);
  } catch {
    // Ignore storage failures.
  }
}

export function clearStoredToken() {
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Ignore storage failures.
  }
}
