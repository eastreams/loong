const STORAGE_KEY = "loong-web-token";

export function getStoredToken(): string | null {
  try {
    return sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

export function setStoredToken(token: string) {
  try {
    sessionStorage.setItem(STORAGE_KEY, token);
  } catch {
    // Ignore storage failures.
  }
}

export function clearStoredToken() {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // Ignore storage failures.
  }
}
