import { useEffect, useRef, useState, type FormEvent } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../../lib/api/client";
import { resolveTokenHintEnv, resolveTokenHintPath } from "../../../lib/auth/tokenHint";
import { onboardingApi } from "../../onboarding/api";
import {
  buildPreferencesSavePayload,
  buildProviderSavePayload,
  readProviderSaveError,
  readProviderValidationFailure,
  usePreferencesForm,
  useProviderConfigForm,
} from "../../onboarding/provider/providerConfig";
import {
  type DashboardApprovals,
  dashboardApi,
  type DashboardConnectivity,
  type DashboardDebugConsole,
  type DashboardConfigSnapshot,
  type DashboardProviderItem,
  type DashboardRuntime,
  type DashboardSummary,
  type DashboardTools,
} from "../api";
import type { WebSessionContextValue } from "../../../contexts/WebSessionContext";

type SettingsModalPhase = "pending" | "success" | "error";

interface SettingsModalState {
  phase: SettingsModalPhase;
  title: string;
  body: string;
}

interface DashboardSnapshot {
  summary: DashboardSummary;
  providers: DashboardProviderItem[];
  runtime: DashboardRuntime;
  connectivity: DashboardConnectivity;
  config: DashboardConfigSnapshot;
  tools: DashboardTools;
  approvals: DashboardApprovals;
}

interface DashboardPreferencesSourceOverride {
  promptAddendum?: string;
}

interface DashboardReloadOptions {
  forceFormReset?: boolean;
  preferencesSourceOverride?: DashboardPreferencesSourceOverride;
}

function wait(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function readDashboardError(
  error: unknown,
  t: TFunction,
  markUnauthorized: () => void,
  authMode: string | null,
  tokenPath: string | null,
  tokenEnv: string | null,
): string {
  if (error instanceof ApiRequestError && error.status === 401) {
    markUnauthorized();
    return authMode === "same_origin_session"
      ? t("auth.sessionInvalidBody")
      : t("auth.invalidBody", {
          tokenPath: resolveTokenHintPath(tokenPath),
          tokenEnv: resolveTokenHintEnv(tokenEnv),
        });
  }

  return error instanceof Error ? error.message : "Failed to load dashboard";
}

interface UseDashboardDataParams {
  t: TFunction;
  connection: WebSessionContextValue;
  providerForm: ReturnType<typeof useProviderConfigForm>;
  preferencesForm: ReturnType<typeof usePreferencesForm>;
}

export function useDashboardData({
  t,
  connection,
  providerForm,
  preferencesForm,
}: UseDashboardDataParams) {
  const {
    canAccessProtectedApi,
    acceptValidatedOnboardingStatus,
    refreshOnboardingStatus,
    authRevision,
    markUnauthorized,
    status,
    authMode,
    tokenPath,
    tokenEnv,
  } = connection;

  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [providers, setProviders] = useState<DashboardProviderItem[]>([]);
  const [runtime, setRuntime] = useState<DashboardRuntime | null>(null);
  const [connectivity, setConnectivity] = useState<DashboardConnectivity | null>(null);
  const [config, setConfig] = useState<DashboardConfigSnapshot | null>(null);
  const [tools, setTools] = useState<DashboardTools | null>(null);
  const [approvals, setApprovals] = useState<DashboardApprovals | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsNotice, setSettingsNotice] = useState<string | null>(null);
  const [isSavingSettings, setIsSavingSettings] = useState(false);
  const [isRefreshingDiagnostics, setIsRefreshingDiagnostics] = useState(false);
  const [settingsModal, setSettingsModal] = useState<SettingsModalState | null>(null);
  const [showDebugConsole, setShowDebugConsole] = useState(false);
  const [debugConsole, setDebugConsole] = useState<DashboardDebugConsole | null>(null);
  const [debugConsoleError, setDebugConsoleError] = useState<string | null>(null);
  const dashboardLoadGenerationRef = useRef(0);

  function buildProviderFormSource(snapshot: DashboardSnapshot) {
    return {
      kind: snapshot.config.activeProvider ?? snapshot.config.lastProvider ?? "",
      model: snapshot.config.model ?? "",
      baseUrlOrEndpoint: snapshot.config.providerEndpointExplicit
        ? snapshot.config.endpoint ?? ""
        : snapshot.config.providerBaseUrl ?? "",
      apiKeyConfigured: snapshot.config.apiKeyConfigured ?? false,
    };
  }

  function buildPreferencesFormSource(
    snapshot: DashboardSnapshot,
    options?: DashboardPreferencesSourceOverride,
  ) {
    return {
      personality: snapshot.config.personality || "calm_engineering",
      memoryProfile: snapshot.config.memoryProfile || "window_only",
      slidingWindow: snapshot.config.slidingWindow ?? 12,
      promptAddendum:
        options?.promptAddendum ?? snapshot.config.promptAddendum ?? "",
    };
  }

  function commitDashboardSnapshot(
    snapshot: DashboardSnapshot,
    options?: DashboardReloadOptions,
  ) {
    const forceFormReset = options?.forceFormReset ?? false;

    setSummary(snapshot.summary);
    setProviders(snapshot.providers);
    setRuntime(snapshot.runtime);
    setConnectivity(snapshot.connectivity);
    setConfig(snapshot.config);
    setTools(snapshot.tools);
    setApprovals(snapshot.approvals);
    providerForm.resetFromSource(buildProviderFormSource(snapshot), {
      force: forceFormReset,
    });
    preferencesForm.resetFromSource(
      buildPreferencesFormSource(snapshot, options?.preferencesSourceOverride),
      {
        force: forceFormReset,
      },
    );
  }

  async function fetchDashboardSnapshot(): Promise<DashboardSnapshot> {
    const [
      loadedSummary,
      loadedProviders,
      loadedRuntime,
      loadedConnectivity,
      loadedConfig,
      loadedTools,
      loadedApprovals,
    ] = await Promise.all([
      dashboardApi.loadSummary(),
      dashboardApi.loadProviders(),
      dashboardApi.loadRuntime(),
      dashboardApi.loadConnectivity(),
      dashboardApi.loadConfig(),
      dashboardApi.loadTools(),
      dashboardApi.loadApprovals(),
    ]);

    return {
      summary: loadedSummary,
      providers: loadedProviders.items,
      runtime: loadedRuntime,
      connectivity: loadedConnectivity,
      config: loadedConfig,
      tools: loadedTools,
      approvals: loadedApprovals,
    };
  }

  async function reloadDashboardData(options?: DashboardReloadOptions) {
    setError(null);
    const generation = ++dashboardLoadGenerationRef.current;
    const snapshot = await fetchDashboardSnapshot();
    if (generation !== dashboardLoadGenerationRef.current) {
      return null;
    }

    commitDashboardSnapshot(snapshot, options);
    return snapshot;
  }

  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      dashboardLoadGenerationRef.current += 1;
      setSummary(null);
      setProviders([]);
      setRuntime(null);
      setConnectivity(null);
      setConfig(null);
      setTools(null);
      setApprovals(null);
      setError(
        status === "unauthorized"
          ? authMode === "same_origin_session"
            ? t("auth.sessionInvalidBody")
            : t("auth.invalidBody", {
                tokenPath: tokenPath ?? "",
                tokenEnv: tokenEnv ?? "LOONG_WEB_TOKEN",
              })
          : t("auth.requiredBody"),
      );
      return () => {
        cancelled = true;
      };
    }

    async function loadDashboardSnapshot() {
      try {
        await reloadDashboardData();
      } catch (loadError) {
        if (!cancelled) {
          setError(
            readDashboardError(
              loadError,
              t,
              markUnauthorized,
              authMode,
              tokenPath,
              tokenEnv,
            ),
          );
        }
      }
    }

    void loadDashboardSnapshot();

    return () => {
      cancelled = true;
      dashboardLoadGenerationRef.current += 1;
    };
  }, [
    authMode,
    authRevision,
    canAccessProtectedApi,
    markUnauthorized,
    status,
    t,
    tokenEnv,
    tokenPath,
  ]);

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;

    if (!showDebugConsole || !canAccessProtectedApi) {
      return () => {
        cancelled = true;
        if (timer) {
          window.clearInterval(timer);
        }
      };
    }

    async function refreshDebugConsole() {
      try {
        const payload = await dashboardApi.loadDebugConsole();
        if (!cancelled) {
          setDebugConsole(payload);
          setDebugConsoleError(null);
        }
      } catch (loadError) {
        if (!cancelled) {
          setDebugConsoleError(
            readDashboardError(
              loadError,
              t,
              markUnauthorized,
              authMode,
              tokenPath,
              tokenEnv,
            ),
          );
        }
      }
    }

    void refreshDebugConsole();
    timer = window.setInterval(() => {
      void refreshDebugConsole();
    }, 4000);

    return () => {
      cancelled = true;
      if (timer) {
        window.clearInterval(timer);
      }
    };
  }, [
    authMode,
    canAccessProtectedApi,
    markUnauthorized,
    showDebugConsole,
    t,
    tokenEnv,
    tokenPath,
  ]);

  async function handleRefreshDiagnostics() {
    setSettingsError(null);
    setSettingsNotice(null);
    setIsRefreshingDiagnostics(true);
    try {
      const snapshot = await reloadDashboardData();
      if (snapshot) {
        setSettingsNotice(t("dashboard.settings.refreshed"));
      }
    } catch (loadError) {
      setSettingsError(
        readDashboardError(
          loadError,
          t,
          markUnauthorized,
          authMode,
          tokenPath,
          tokenEnv,
        ),
      );
    } finally {
      setIsRefreshingDiagnostics(false);
    }
  }

  async function handleApplySettings(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSettingsError(null);
    setSettingsNotice(null);
    setIsSavingSettings(true);
    let providerApplied = false;
    try {
      const savedPreferences = buildPreferencesSavePayload({
        personality: preferencesForm.personality,
        memoryProfile: preferencesForm.memoryProfile,
        slidingWindow: preferencesForm.slidingWindow,
        promptAddendum: preferencesForm.promptAddendum,
      }, t);
      setSettingsModal({
        phase: "pending",
        title: t("dashboard.settings.applyPending"),
        body: t("onboarding.validation.pending"),
      });
      const result = await onboardingApi.applyProvider(
        buildProviderSavePayload({
          kind: providerForm.kind,
          model: providerForm.model,
          baseUrlOrEndpoint: providerForm.baseUrlOrEndpoint,
          apiKey: providerForm.apiKey,
        }),
      );
      if (result.passed) {
        providerApplied = true;
        acceptValidatedOnboardingStatus(result.status);
        await onboardingApi.savePreferences(savedPreferences);
        refreshOnboardingStatus();
        await reloadDashboardData({
          forceFormReset: true,
          preferencesSourceOverride: {
            promptAddendum: savedPreferences.promptAddendum ?? "",
          },
        });
        setSettingsModal({
          phase: "success",
          title: t("dashboard.settings.saved"),
          body: t("onboarding.validation.success"),
        });
        setSettingsNotice(t("dashboard.settings.saved"));
        await wait(1100);
      } else {
        const validationError = readProviderValidationFailure(result.credentialStatus, t);
        setSettingsModal({
          phase: "error",
          title: t("onboarding.validation.failed"),
          body: validationError,
        });
        await reloadDashboardData({ forceFormReset: true });
        setSettingsError(validationError);
        await wait(1600);
      }
    } catch (saveError) {
      if (saveError instanceof ApiRequestError && saveError.status === 401) {
        markUnauthorized();
      }
      if (providerApplied) {
        refreshOnboardingStatus();
        try {
          await reloadDashboardData({ forceFormReset: true });
        } catch {
          // Keep the original save error visible if the recovery refresh also fails.
        }
      }

      const saveErrorMessage = providerApplied
        ? t("dashboard.settings.preferencesSaveFailed", {
            defaultValue:
              "Provider settings were applied, but assistant settings could not be saved.",
          })
        : readProviderSaveError(saveError, t, "dashboard.settings.saveFailed");
      setSettingsError(saveErrorMessage);
      setSettingsModal({
        phase: "error",
        title: t("dashboard.settings.saveFailed"),
        body: saveErrorMessage,
      });
      await wait(1600);
    } finally {
      setSettingsModal(null);
      setIsSavingSettings(false);
    }
  }

  return {
    state: {
      summary,
      providers,
      runtime,
      connectivity,
      config,
      tools,
      approvals,
      error,
      settingsError,
      settingsNotice,
      isSavingSettings,
      isRefreshingDiagnostics,
      settingsModal,
      showDebugConsole,
      debugConsole,
      debugConsoleError,
    },
    actions: {
      setShowDebugConsole,
      handleRefreshDiagnostics,
      handleApplySettings,
    },
  };
}
