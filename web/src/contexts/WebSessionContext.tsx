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
import { onboardingApi } from "../features/onboarding/api";

const ONBOARDING_VALIDATION_STORAGE_KEY = "loongclaw.onboarding.validation-key";

interface MetaAuthInfo {
  required: boolean;
  scheme: string;
  header: string;
  tokenPath: string;
  tokenEnv: string;
}

export interface OnboardingStatus {
  runtimeOnline: boolean;
  tokenRequired: boolean;
  tokenPaired: boolean;
  configExists: boolean;
  configLoadable: boolean;
  providerConfigured: boolean;
  providerReachable: boolean;
  activeProvider: string | null;
  activeModel: string;
  providerBaseUrl: string;
  providerEndpoint: string;
  apiKeyConfigured: boolean;
  personality: string;
  memoryProfile: string;
  promptAddendum: string;
  configPath: string;
  blockingStage:
    | "runtime_offline"
    | "token_pairing"
    | "missing_config"
    | "config_invalid"
    | "provider_setup"
    | "provider_unreachable"
    | "ready"
    | string;
  nextAction: string;
}

export interface WebSessionContextValue {
  endpoint: string;
  status: "connected" | "auth_required" | "unauthorized";
  authRequired: boolean;
  canAccessProtectedApi: boolean;
  onboardingLoading: boolean;
  onboardingStatus: OnboardingStatus | null;
  onboardingBlocked: boolean;
  onboardingValidationSatisfied: boolean;
  acknowledgeOnboarding: () => void;
  markOnboardingValidated: () => void;
  acceptValidatedOnboardingStatus: (status: OnboardingStatus) => void;
  clearOnboardingValidation: () => void;
  refreshOnboardingStatus: () => void;
  autoPairingInProgress: boolean;
  tokenPath: string | null;
  tokenEnv: string | null;
  authRevision: number;
  saveToken: (token: string) => void;
  clearToken: () => void;
  markUnauthorized: () => void;
}

export const WebSessionContext = createContext<WebSessionContextValue | null>(null);

function buildOnboardingValidationKey(status: OnboardingStatus | null): string | null {
  if (!status?.tokenPaired || !status.configLoadable || !status.providerConfigured) {
    return null;
  }

  return [
    status.activeProvider ?? "none",
    status.activeModel,
    status.providerBaseUrl,
    status.providerEndpoint,
    status.configPath,
  ].join("|");
}

export function WebSessionProvider({ children }: PropsWithChildren) {
  const [authInfo, setAuthInfo] = useState<MetaAuthInfo | null>(null);
  const [storedToken, setTokenState] = useState<string | null>(() => getStoredToken());
  const [isUnauthorized, setIsUnauthorized] = useState(false);
  const [authRevision, setAuthRevision] = useState(0);
  const [onboardingStatus, setOnboardingStatus] = useState<OnboardingStatus | null>(null);
  const [onboardingLoading, setOnboardingLoading] = useState(true);
  const [onboardingAcknowledged, setOnboardingAcknowledged] = useState(false);
  const [onboardingRevision, setOnboardingRevision] = useState(0);
  const [autoPairingInProgress, setAutoPairingInProgress] = useState(false);
  const [autoPairingAttempted, setAutoPairingAttempted] = useState(false);
  const [validatedOnboardingKey, setValidatedOnboardingKey] = useState<string | null>(() => {
    if (typeof window === "undefined") {
      return null;
    }
    return window.sessionStorage.getItem(ONBOARDING_VALIDATION_STORAGE_KEY);
  });
  const authRequired = authInfo?.required ?? true;

  function persistOnboardingValidationKey(key: string | null) {
    if (typeof window === "undefined") {
      return;
    }

    if (key) {
      window.sessionStorage.setItem(ONBOARDING_VALIDATION_STORAGE_KEY, key);
    } else {
      window.sessionStorage.removeItem(ONBOARDING_VALIDATION_STORAGE_KEY);
    }
    setValidatedOnboardingKey(key);
  }

  useEffect(() => {
    let cancelled = false;

    async function loadMeta() {
      try {
        const response = await fetch(`${getApiBaseUrl()}/api/meta`, {
          credentials: "include",
        });
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

  useEffect(() => {
    let cancelled = false;

    async function loadOnboardingStatus() {
      setOnboardingLoading(true);
      try {
        const headers = new Headers();
        if (storedToken?.trim()) {
          headers.set("Authorization", `Bearer ${storedToken.trim()}`);
        }

        const response = await fetch(`${getApiBaseUrl()}/api/onboard/status`, {
          credentials: "include",
          headers,
        });
        const payload = await response.json().catch(() => null);

        if (cancelled) {
          return;
        }

        if (payload?.data) {
          setOnboardingStatus(payload.data as OnboardingStatus);
        } else {
          setOnboardingStatus(null);
        }
      } catch {
        if (!cancelled) {
          setOnboardingStatus({
            runtimeOnline: false,
            tokenRequired: true,
            tokenPaired: false,
            configExists: false,
            configLoadable: false,
            providerConfigured: false,
            providerReachable: false,
            activeProvider: null,
            activeModel: "",
            providerBaseUrl: "",
            providerEndpoint: "",
            apiKeyConfigured: false,
            personality: "calm_engineering",
            memoryProfile: "window_only",
            promptAddendum: "",
            configPath: "",
            blockingStage: "runtime_offline",
            nextAction: "start_local_runtime",
          });
        }
      } finally {
        if (!cancelled) {
          setOnboardingLoading(false);
        }
      }
    }

    void loadOnboardingStatus();
    return () => {
      cancelled = true;
    };
  }, [authRevision, onboardingRevision, storedToken]);

  useEffect(() => {
    setOnboardingAcknowledged(false);
  }, [authRevision]);

  useEffect(() => {
    if (storedToken?.trim() || onboardingStatus?.tokenPaired) {
      setAutoPairingAttempted(false);
    }
  }, [storedToken, onboardingStatus?.tokenPaired]);

  useEffect(() => {
    if (
      onboardingLoading ||
      autoPairingAttempted ||
      autoPairingInProgress ||
      !!storedToken?.trim() ||
      onboardingStatus?.tokenPaired ||
      !authRequired
    ) {
      return;
    }

    let cancelled = false;

    async function tryAutoPair() {
      setAutoPairingInProgress(true);
      try {
        const result = await onboardingApi.autoPair();
        if (cancelled) {
          return;
        }
        if (result.paired) {
          setIsUnauthorized(false);
          setAuthRevision((current) => current + 1);
          setOnboardingRevision((current) => current + 1);
        }
      } catch {
        // Fall back to manual token entry when lightweight pairing is unavailable.
      } finally {
        if (!cancelled) {
          setAutoPairingInProgress(false);
          setAutoPairingAttempted(true);
        }
      }
    }

    void tryAutoPair();
    return () => {
      cancelled = true;
    };
  }, [
    authRequired,
    autoPairingAttempted,
    autoPairingInProgress,
    onboardingLoading,
    onboardingStatus?.tokenPaired,
    storedToken,
  ]);

  const hasToken = !!storedToken?.trim();
  const tokenPaired = onboardingStatus?.tokenPaired ?? hasToken;
  const currentOnboardingValidationKey =
    buildOnboardingValidationKey(onboardingStatus);
  const onboardingValidationSatisfied =
    currentOnboardingValidationKey != null &&
    validatedOnboardingKey === currentOnboardingValidationKey;
  const status: WebSessionContextValue["status"] = authRequired
    ? isUnauthorized
      ? "unauthorized"
      : tokenPaired
        ? "connected"
        : "auth_required"
    : "connected";

  const value = useMemo<WebSessionContextValue>(
    () => ({
      endpoint: getApiBaseUrl(),
      status,
      authRequired,
      canAccessProtectedApi: !authRequired || (tokenPaired && !isUnauthorized),
      onboardingLoading,
      onboardingStatus,
      onboardingBlocked:
        onboardingLoading ||
        (onboardingStatus?.blockingStage ?? "ready") !== "ready" ||
        !onboardingValidationSatisfied ||
        !onboardingAcknowledged,
      onboardingValidationSatisfied,
      acknowledgeOnboarding: () => {
        setOnboardingAcknowledged(true);
      },
      markOnboardingValidated: () => {
        if (!currentOnboardingValidationKey || typeof window === "undefined") {
          return;
        }
        persistOnboardingValidationKey(currentOnboardingValidationKey);
      },
      acceptValidatedOnboardingStatus: (status) => {
        const nextKey = buildOnboardingValidationKey(status);
        setOnboardingStatus(status);
        setIsUnauthorized(false);
        persistOnboardingValidationKey(nextKey);
      },
      clearOnboardingValidation: () => {
        persistOnboardingValidationKey(null);
        setOnboardingAcknowledged(false);
      },
      refreshOnboardingStatus: () => {
        setOnboardingRevision((current) => current + 1);
      },
      autoPairingInProgress,
      tokenPath: authInfo?.tokenPath ?? null,
      tokenEnv: authInfo?.tokenEnv ?? null,
      authRevision,
      saveToken: (token: string) => {
        const normalized = token.trim();
        setStoredToken(normalized);
        setTokenState(normalized);
        setIsUnauthorized(false);
        setAutoPairingAttempted(true);
        setAuthRevision((current) => current + 1);
      },
      clearToken: () => {
        void onboardingApi.clearPairing().catch(() => {});
        clearStoredToken();
        setTokenState(null);
        setIsUnauthorized(false);
        setAutoPairingAttempted(true);
        setAuthRevision((current) => current + 1);
      },
      markUnauthorized: () => {
        setIsUnauthorized(true);
      },
    }),
    [
      authInfo?.tokenEnv,
      authInfo?.tokenPath,
      authRequired,
      authRevision,
      autoPairingInProgress,
      isUnauthorized,
      onboardingLoading,
      onboardingStatus,
      status,
      tokenPaired,
      onboardingAcknowledged,
      onboardingValidationSatisfied,
      onboardingRevision,
      currentOnboardingValidationKey,
      validatedOnboardingKey,
    ],
  );

  return (
    <WebSessionContext.Provider value={value}>
      {children}
    </WebSessionContext.Provider>
  );
}
