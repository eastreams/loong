import { useEffect, useMemo, useState } from "react";
import { isApiAbortError, isApiRequestError } from "../lib/api/client";
import { getApiBaseUrl } from "../lib/config/env";
import {
  clearStoredToken,
  getStoredToken,
  setStoredToken as setRawStoredToken,
} from "../lib/auth/tokenStore";
import {
  onboardingApi,
  type MetaAuthInfo,
  type OnboardingStatus,
} from "../features/onboarding/api";
import type { WebSessionContextValue } from "../contexts/WebSessionContext";

const ONBOARDING_VALIDATION_STORAGE_KEY = "loongclaw.onboarding.validation-key";
const ONBOARDING_ACK_STORAGE_KEY = "loongclaw.onboarding.ack-key";

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

function buildOfflineOnboardingStatus(authMode: string | null): OnboardingStatus {
  return {
    runtimeOnline: false,
    tokenRequired: authMode !== "same_origin_session",
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
  };
}

function buildUnauthorizedOnboardingStatus(authMode: string | null): OnboardingStatus {
  const sameOriginSession = authMode === "same_origin_session";
  return {
    runtimeOnline: true,
    tokenRequired: !sameOriginSession,
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
    blockingStage: sameOriginSession ? "session_refresh" : "token_pairing",
    nextAction: sameOriginSession ? "refresh_local_session" : "enter_local_token",
  };
}

export function useWebSessionManager() {
  const [authInfo, setAuthInfo] = useState<MetaAuthInfo | null>(null);
  const [storedToken, setTokenState] = useState<string | null>(() => getStoredToken());
  const [isUnauthorized, setIsUnauthorized] = useState(false);
  const [authRevision, setAuthRevision] = useState(0);
  const [onboardingStatus, setOnboardingStatus] = useState<OnboardingStatus | null>(null);
  const [onboardingLoading, setOnboardingLoading] = useState(true);
  const [onboardingRevision, setOnboardingRevision] = useState(0);
  const [autoPairingInProgress, setAutoPairingInProgress] = useState(false);
  const [autoPairingAttempted, setAutoPairingAttempted] = useState(false);
  const [validatedOnboardingKey, setValidatedOnboardingKey] = useState<string | null>(() => {
    if (typeof window === "undefined") {
      return null;
    }
    return window.sessionStorage.getItem(ONBOARDING_VALIDATION_STORAGE_KEY);
  });
  const [acknowledgedOnboardingKey, setAcknowledgedOnboardingKey] = useState<string | null>(() => {
    if (typeof window === "undefined") {
      return null;
    }
    return window.sessionStorage.getItem(ONBOARDING_ACK_STORAGE_KEY);
  });

  const authRequired = authInfo?.required ?? true;
  const authMode = authInfo?.mode ?? null;

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

  function persistOnboardingAcknowledgedKey(key: string | null) {
    if (typeof window === "undefined") {
      return;
    }

    if (key) {
      window.sessionStorage.setItem(ONBOARDING_ACK_STORAGE_KEY, key);
    } else {
      window.sessionStorage.removeItem(ONBOARDING_ACK_STORAGE_KEY);
    }
    setAcknowledgedOnboardingKey(key);
  }

  useEffect(() => {
    const controller = new AbortController();

    async function loadMeta() {
      try {
        const meta = await onboardingApi.loadMeta({
          signal: controller.signal,
          skipAuth: true,
        });
        setAuthInfo(meta.auth);
      } catch (error) {
        if (!isApiAbortError(error)) {
          setAuthInfo(null);
        }
      }
    }

    void loadMeta();
    return () => {
      controller.abort();
    };
  }, []);

  useEffect(() => {
    const controller = new AbortController();

    async function loadOnboardingStatus() {
      setOnboardingLoading(true);
      try {
        const status = await onboardingApi.loadStatus({
          signal: controller.signal,
          authToken: storedToken?.trim() ?? "",
        });
        setIsUnauthorized(false);
        setOnboardingStatus(status);
      } catch (error) {
        if (!isApiAbortError(error)) {
          if (isApiRequestError(error) && error.status === 401) {
            setIsUnauthorized(true);
            setOnboardingStatus(buildUnauthorizedOnboardingStatus(authMode));
          } else {
            setOnboardingStatus(buildOfflineOnboardingStatus(authMode));
          }
        }
      } finally {
        if (!controller.signal.aborted) {
          setOnboardingLoading(false);
        }
      }
    }

    void loadOnboardingStatus();
    return () => {
      controller.abort();
    };
  }, [authMode, authRevision, onboardingRevision, storedToken]);

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
      !authRequired ||
      authMode === "same_origin_session"
    ) {
      return;
    }

    const controller = new AbortController();

    async function tryAutoPair() {
      setAutoPairingInProgress(true);
      try {
        const result = await onboardingApi.autoPair({
          signal: controller.signal,
        });
        if (result.paired) {
          setIsUnauthorized(false);
          setAuthRevision((current) => current + 1);
          setOnboardingRevision((current) => current + 1);
        }
      } catch (error) {
        if (isApiAbortError(error)) {
          return;
        }
      } finally {
        if (!controller.signal.aborted) {
          setAutoPairingInProgress(false);
          setAutoPairingAttempted(true);
        }
      }
    }

    void tryAutoPair();
    return () => {
      controller.abort();
    };
  }, [
    authRequired,
    authMode,
    autoPairingAttempted,
    autoPairingInProgress,
    onboardingLoading,
    onboardingStatus?.tokenPaired,
    storedToken,
  ]);

  const hasToken = !!storedToken?.trim();
  const tokenPaired = onboardingStatus?.tokenPaired ?? hasToken;
  const currentOnboardingValidationKey = buildOnboardingValidationKey(onboardingStatus);
  const onboardingValidationSatisfied =
    currentOnboardingValidationKey != null &&
    validatedOnboardingKey === currentOnboardingValidationKey;
  const onboardingAcknowledged =
    currentOnboardingValidationKey != null &&
    acknowledgedOnboardingKey === currentOnboardingValidationKey;
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
        if (!currentOnboardingValidationKey) {
          return;
        }
        persistOnboardingAcknowledgedKey(currentOnboardingValidationKey);
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
        persistOnboardingAcknowledgedKey(null);
      },
      refreshOnboardingStatus: () => {
        setOnboardingRevision((current) => current + 1);
      },
      restartOnboarding: () => {
        persistOnboardingValidationKey(null);
        persistOnboardingAcknowledgedKey(null);
        setOnboardingStatus((current) =>
          current
            ? {
                ...current,
                blockingStage: "provider_setup",
                nextAction: "configure_provider",
              }
            : current,
        );
      },
      autoPairingInProgress,
      tokenPath: authInfo?.tokenPath ?? null,
      tokenEnv: authInfo?.tokenEnv ?? null,
      authMode,
      authRevision,
      saveToken: (token: string) => {
        const normalized = token.trim();
        setRawStoredToken(normalized);
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
      authMode,
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
      acknowledgedOnboardingKey,
      validatedOnboardingKey,
    ],
  );

  return value;
}

