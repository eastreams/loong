export function safeLocalStorageGet(key: string) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
