const DEFAULT_API_BASE_URL = "http://127.0.0.1:4317";
const DEV_WEB_PORT = "4173";
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);

export type ApiBaseUrlSource =
  | "explicit"
  | "dev_loopback"
  | "window_origin"
  | "default_loopback";

export interface ApiBaseUrlResolution {
  baseUrl: string;
  source: ApiBaseUrlSource;
}

function normalizeApiBaseUrl(baseUrl: string): string {
  return baseUrl.trim().replace(/\/+$/, "");
}

export function resolveApiBaseUrl(): ApiBaseUrlResolution {
  const explicitBaseUrl = import.meta.env.VITE_API_BASE_URL;
  if (typeof explicitBaseUrl === "string" && explicitBaseUrl.trim()) {
    return {
      baseUrl: normalizeApiBaseUrl(explicitBaseUrl),
      source: "explicit",
    };
  }

  if (typeof window !== "undefined") {
    const { protocol, hostname, port, origin } = window.location;
    if (LOOPBACK_HOSTS.has(hostname) && port === DEV_WEB_PORT) {
      return {
        baseUrl: DEFAULT_API_BASE_URL,
        source: "dev_loopback",
      };
    }

    if (protocol === "http:" || protocol === "https:") {
      return {
        baseUrl: normalizeApiBaseUrl(origin),
        source: "window_origin",
      };
    }
  }

  return {
    baseUrl: DEFAULT_API_BASE_URL,
    source: "default_loopback",
  };
}

export function getApiBaseUrl() {
  return resolveApiBaseUrl().baseUrl;
}
