import {
  createContext,
  useEffect,
  useMemo,
  useState,
  type PropsWithChildren,
} from "react";
import { getApiBaseUrl } from "../lib/config/env";
import {
  clearStoredToken,
  getStoredToken,
  setStoredToken,
} from "../lib/auth/tokenStore";

interface MetaAuthInfo {
  required: boolean;
  scheme: string;
  header: string;
  tokenPath: string;
  tokenEnv: string;
}

export interface WebSessionContextValue {
  endpoint: string;
  status: "connected" | "auth_required" | "unauthorized";
  authRequired: boolean;
  canAccessProtectedApi: boolean;
  tokenPath: string | null;
  tokenEnv: string | null;
  authRevision: number;
  saveToken: (token: string) => void;
  clearToken: () => void;
  markUnauthorized: () => void;
}

export const WebSessionContext = createContext<WebSessionContextValue | null>(null);

export function WebSessionProvider({ children }: PropsWithChildren) {
  const [authInfo, setAuthInfo] = useState<MetaAuthInfo | null>(null);
  const [storedToken, setTokenState] = useState<string | null>(() => getStoredToken());
  const [isUnauthorized, setIsUnauthorized] = useState(false);
  const [authRevision, setAuthRevision] = useState(0);

  useEffect(() => {
    let cancelled = false;

    async function loadMeta() {
      try {
        const response = await fetch(`${getApiBaseUrl()}/api/meta`);
        const payload = await response.json().catch(() => null);
        if (cancelled || !payload?.data?.auth) {
          return;
        }
        setAuthInfo(payload.data.auth as MetaAuthInfo);
      } catch {
        if (!cancelled) {
          setAuthInfo(null);
        }
      }
    }

    void loadMeta();
    return () => {
      cancelled = true;
    };
  }, []);

  const authRequired = authInfo?.required ?? true;
  const hasToken = !!storedToken?.trim();
  const status: WebSessionContextValue["status"] = authRequired
    ? isUnauthorized
      ? "unauthorized"
      : hasToken
        ? "connected"
        : "auth_required"
    : "connected";

  const value = useMemo<WebSessionContextValue>(
    () => ({
      endpoint: getApiBaseUrl(),
      status,
      authRequired,
      canAccessProtectedApi: !authRequired || (hasToken && !isUnauthorized),
      tokenPath: authInfo?.tokenPath ?? null,
      tokenEnv: authInfo?.tokenEnv ?? null,
      authRevision,
      saveToken: (token: string) => {
        const normalized = token.trim();
        setStoredToken(normalized);
        setTokenState(normalized);
        setIsUnauthorized(false);
        setAuthRevision((current) => current + 1);
      },
      clearToken: () => {
        clearStoredToken();
        setTokenState(null);
        setIsUnauthorized(false);
        setAuthRevision((current) => current + 1);
      },
      markUnauthorized: () => {
        setIsUnauthorized(true);
      },
    }),
    [authInfo?.tokenEnv, authInfo?.tokenPath, authRequired, authRevision, hasToken, isUnauthorized, status],
  );

  return (
    <WebSessionContext.Provider value={value}>
      {children}
    </WebSessionContext.Provider>
  );
}
