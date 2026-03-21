export function getApiBaseUrl() {
  const explicitBaseUrl = import.meta.env.VITE_API_BASE_URL;
  if (explicitBaseUrl) {
    return explicitBaseUrl;
  }

  if (typeof window !== "undefined") {
    const { protocol, hostname, port, origin } = window.location;
    if (
      (hostname === "127.0.0.1" || hostname === "localhost" || hostname === "::1") &&
      port === "4173"
    ) {
      return "http://127.0.0.1:4317";
    }
    if (protocol === "http:" || protocol === "https:") {
      return origin;
    }
  }

  return "http://127.0.0.1:4317";
}
