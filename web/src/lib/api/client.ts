import { getStoredToken } from "../auth/tokenStore";
import {
  resolveApiBaseUrl,
  type ApiBaseUrlResolution,
  type ApiBaseUrlSource,
} from "../config/env";
import type { ApiEnvelope } from "./types";

export interface ApiRequestOptions extends Omit<RequestInit, "signal"> {
  signal?: AbortSignal;
  timeoutMs?: number;
  authToken?: string | null;
  skipAuth?: boolean;
}

export type ApiRequestErrorKind = "http" | "network" | "timeout" | "aborted";

export class ApiRequestError extends Error {
  status: number;
  code?: string;
  method: string;
  url: string;
  kind: ApiRequestErrorKind;
  baseUrl: string;
  baseUrlSource: ApiBaseUrlSource;

  constructor(
    message: string,
    status: number,
    code: string | undefined,
    method: string,
    url: string,
    kind: ApiRequestErrorKind,
    baseUrl: string,
    baseUrlSource: ApiBaseUrlSource,
  ) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.code = code;
    this.method = method;
    this.url = url;
    this.kind = kind;
    this.baseUrl = baseUrl;
    this.baseUrlSource = baseUrlSource;
  }
}

export function isApiRequestError(error: unknown): error is ApiRequestError {
  return error instanceof ApiRequestError;
}

export function isApiAbortError(error: unknown): error is ApiRequestError {
  return isApiRequestError(error) && error.kind === "aborted";
}

export function buildApiUrl(path: string): string {
  return `${resolveApiBaseUrl().baseUrl}${path}`;
}

export async function apiGet<T>(path: string, options?: ApiRequestOptions): Promise<T> {
  return apiRequest<T>(path, options);
}

export async function apiGetData<T>(path: string, options?: ApiRequestOptions): Promise<T> {
  const payload = await apiRequest<ApiEnvelope<T>>(path, options);
  return payload.data;
}

export async function apiPost<TResponse, TBody>(
  path: string,
  body: TBody,
  options?: ApiRequestOptions,
): Promise<TResponse> {
  const headers = new Headers(options?.headers);
  if (!headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  return apiRequest<TResponse>(path, {
    ...options,
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
}

export async function apiPostData<TResponse, TBody>(
  path: string,
  body: TBody,
  options?: ApiRequestOptions,
): Promise<TResponse> {
  const payload = await apiPost<ApiEnvelope<TResponse>, TBody>(path, body, options);
  return payload.data;
}

export async function apiDelete(path: string, options?: ApiRequestOptions): Promise<void> {
  await apiRequest<void>(path, {
    ...options,
    method: "DELETE",
  });
}

export async function apiOpenStream(
  path: string,
  options?: ApiRequestOptions,
): Promise<Response> {
  return apiFetch(path, options);
}

async function apiRequest<T>(path: string, options?: ApiRequestOptions): Promise<T> {
  const response = await apiFetch(path, options);
  const payload = await response.json().catch(() => null);
  return payload as T;
}

function extractApiErrorMessage(payload: unknown): string | null {
  return typeof payload === "object" &&
    payload !== null &&
    "error" in payload &&
    typeof payload.error === "object" &&
    payload.error !== null &&
    "message" in payload.error &&
    typeof payload.error.message === "string"
    ? payload.error.message.trim()
    : null;
}

function extractApiErrorCode(payload: unknown): string | undefined {
  return typeof payload === "object" &&
    payload !== null &&
    "error" in payload &&
    typeof payload.error === "object" &&
    payload.error !== null &&
    "code" in payload.error &&
    typeof payload.error.code === "string"
    ? payload.error.code
    : undefined;
}

function describeApiBaseUrlResolution(resolution: ApiBaseUrlResolution): string {
  switch (resolution.source) {
    case "explicit":
      return `API base URL resolved from VITE_API_BASE_URL: ${resolution.baseUrl}.`;
    case "dev_loopback":
      return `API base URL auto-mapped from the local Vite dev server to ${resolution.baseUrl}.`;
    case "window_origin":
      return `API base URL resolved from the current page origin: ${resolution.baseUrl}.`;
    case "default_loopback":
      return `API base URL fell back to the default loopback address: ${resolution.baseUrl}.`;
  }
}

function describeApiFailure(
  kind: ApiRequestErrorKind,
  status: number,
  payload: unknown,
  method: string,
  url: string,
  resolution: ApiBaseUrlResolution,
): string {
  const payloadMessage = extractApiErrorMessage(payload);
  const requestTarget = `${method} ${url}`;
  const baseUrlHint = describeApiBaseUrlResolution(resolution);

  if (payloadMessage) {
    return `${payloadMessage} (${requestTarget})`;
  }

  if (kind === "timeout") {
    return `Request timed out (${requestTarget}). Check that the Web API is running and reachable from this browser. ${baseUrlHint}`;
  }

  if (kind === "aborted") {
    return `Request cancelled (${requestTarget}).`;
  }

  if (kind === "network") {
    return `Unable to reach the Web API (${requestTarget}). ${baseUrlHint}`;
  }

  if (status === 401) {
    return `Request failed: 401 (${requestTarget}). Authentication failed. Check your local token or session and try again.`;
  }

  if (status === 403) {
    return `Request failed: 403 (${requestTarget}). This browser origin or token is not allowed to perform the request.`;
  }

  if (status === 404) {
    return `Request failed: 404 (${requestTarget}). Check that the Web API is running and that the browser is pointing to the correct API base URL. ${baseUrlHint}`;
  }

  return `Request failed: ${status} (${requestTarget})`;
}

function isAbortError(error: unknown): boolean {
  return (
    (typeof DOMException !== "undefined" &&
      error instanceof DOMException &&
      error.name === "AbortError") ||
    (typeof error === "object" &&
      error !== null &&
      "name" in error &&
      error.name === "AbortError")
  );
}

function createRequestSignal(signal?: AbortSignal, timeoutMs?: number) {
  const controller = new AbortController();
  let timeoutId: ReturnType<typeof setTimeout> | null = null;
  let timedOut = false;

  const abortFromSource = () => {
    controller.abort();
  };

  if (signal) {
    if (signal.aborted) {
      controller.abort();
    } else {
      signal.addEventListener("abort", abortFromSource, { once: true });
    }
  }

  if (typeof timeoutMs === "number" && timeoutMs > 0) {
    timeoutId = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
  }

  return {
    signal: controller.signal,
    didTimeout: () => timedOut,
    cleanup: () => {
      if (timeoutId) {
        clearTimeout(timeoutId);
      }
      if (signal) {
        signal.removeEventListener("abort", abortFromSource);
      }
    },
  };
}

async function apiFetch(path: string, options?: ApiRequestOptions): Promise<Response> {
  const headers = new Headers(options?.headers);
  const resolvedBaseUrl = resolveApiBaseUrl();
  const token = options?.skipAuth
    ? null
    : options && "authToken" in options
      ? options.authToken
      : getStoredToken();
  if (token && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const method = (options?.method ?? "GET").toUpperCase();
  const url = `${resolvedBaseUrl.baseUrl}${path}`;
  const requestSignal = createRequestSignal(options?.signal, options?.timeoutMs);

  let response: Response;
  try {
    response = await fetch(url, {
      ...options,
      credentials: "include",
      headers,
      signal: requestSignal.signal,
    });
  } catch (error) {
    requestSignal.cleanup();
    const kind: ApiRequestErrorKind = requestSignal.didTimeout()
      ? "timeout"
      : isAbortError(error)
        ? "aborted"
        : "network";
    throw new ApiRequestError(
      describeApiFailure(kind, 0, null, method, url, resolvedBaseUrl),
      0,
      undefined,
      method,
      url,
      kind,
      resolvedBaseUrl.baseUrl,
      resolvedBaseUrl.source,
    );
  }

  requestSignal.cleanup();

  if (!response.ok) {
    const payload = await response.json().catch(() => null);
    throw new ApiRequestError(
      describeApiFailure("http", response.status, payload, method, url, resolvedBaseUrl),
      response.status,
      extractApiErrorCode(payload),
      method,
      url,
      "http",
      resolvedBaseUrl.baseUrl,
      resolvedBaseUrl.source,
    );
  }

  return response;
}
