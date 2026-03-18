import { getApiBaseUrl } from "../config/env";
import { getStoredToken } from "../auth/tokenStore";

export class ApiRequestError extends Error {
  status: number;
  code?: string;

  constructor(message: string, status: number, code?: string) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    this.code = code;
  }
}

export async function apiGet<T>(path: string): Promise<T> {
  return apiRequest<T>(path);
}

export async function apiPost<TResponse, TBody>(
  path: string,
  body: TBody,
): Promise<TResponse> {
  return apiRequest<TResponse>(path, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
}

export async function apiDelete(path: string): Promise<void> {
  await apiRequest<void>(path, {
    method: "DELETE",
  });
}

export async function apiOpenStream(
  path: string,
  init?: RequestInit,
): Promise<Response> {
  return apiFetch(path, init);
}

async function apiRequest<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await apiFetch(path, init);
  const payload = await response.json().catch(() => null);
  return payload as T;
}

async function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers);
  const token = getStoredToken();
  if (token && !headers.has("Authorization")) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const response = await fetch(`${getApiBaseUrl()}${path}`, {
    ...init,
    headers,
  });
  if (!response.ok) {
    const payload = await response.json().catch(() => null);
    const message =
      typeof payload?.error?.message === "string"
        ? payload.error.message
        : `Request failed: ${response.status}`;
    throw new ApiRequestError(message, response.status, payload?.error?.code);
  }

  return response;
}
