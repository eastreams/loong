import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Panel } from "../../../components/surfaces/Panel";
import { ApiRequestError } from "../../../lib/api/client";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { onboardingApi } from "../../onboarding/api";
import { DebugConsolePanel } from "../components/DebugConsolePanel";
import {
  dashboardApi,
  type DashboardConnectivity,
  type DashboardDebugConsole,
  type DashboardConfigSnapshot,
  type DashboardProviderItem,
  type DashboardRuntime,
  type DashboardSummary,
  type DashboardToolItem,
  type DashboardTools,
} from "../api";

type SettingsModalPhase = "pending" | "success" | "error";

interface SettingsModalState {
  phase: SettingsModalPhase;
  title: string;
  body: string;
}

type Tone = "good" | "warn" | "muted";

interface SummaryCard {
  key: string;
  value: string;
  chip: string;
  tone: Tone;
  items: Array<{ label: string; value: string }>;
}

interface ConnectivityPresentation {
  summary: string;
  recommendation: string;
  probe: string;
}

function formatApprovalMode(
  approvalMode: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!approvalMode) {
    return t("dashboard.values.notSet");
  }

  switch (approvalMode) {
    case "disabled":
      return t("dashboard.values.approvalOff");
    case "manual":
      return t("dashboard.values.approvalManual");
    case "auto":
      return t("dashboard.values.approvalAuto");
    default:
      return approvalMode;
  }
}

function formatShellPolicy(
  shellPolicy: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!shellPolicy) {
    return t("dashboard.values.notSet");
  }

  switch (shellPolicy) {
    case "deny":
      return t("dashboard.values.denyByDefault");
    case "allow":
      return t("dashboard.values.allowByDefault");
    default:
      return shellPolicy;
  }
}

function formatPromptMode(
  promptMode: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!promptMode) {
    return t("dashboard.values.notSet");
  }

  switch (promptMode) {
    case "native_prompt_pack":
      return t("dashboard.values.nativePrompt");
    case "inline_prompt":
      return t("dashboard.values.inlinePrompt");
    default:
      return promptMode;
  }
}

function formatPersonality(
  personality: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!personality) {
    return t("dashboard.values.notSet");
  }

  switch (personality) {
    case "calm_engineering":
      return t("dashboard.values.personalityCalmEngineering");
    case "friendly_collab":
      return t("dashboard.values.personalityFriendlyCollab");
    case "autonomous_executor":
      return t("dashboard.values.personalityAutonomousExecutor");
    default:
      return personality;
  }
}

function formatMemoryProfile(
  memoryProfile: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!memoryProfile) {
    return t("dashboard.values.notSet");
  }

  switch (memoryProfile) {
    case "window_only":
      return t("dashboard.values.memoryProfileWindowOnly");
    case "window_plus_summary":
      return t("dashboard.values.memoryProfileWindowPlusSummary");
    case "profile_plus_window":
      return t("dashboard.values.memoryProfileProfilePlusWindow");
    default:
      return memoryProfile;
  }
}

function readDashboardError(
  error: unknown,
  t: ReturnType<typeof useTranslation>["t"],
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
          tokenPath: tokenPath ?? "",
          tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN",
        });
  }

  return error instanceof Error ? error.message : "Failed to load dashboard";
}

function readProviderValidationFailure(
  credentialStatus: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  return t(`onboarding.validation.statuses.${credentialStatus}`, {
    defaultValue: t("onboarding.validation.failed"),
  });
}

function wait(ms: number) {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function buildConnectivityCopy(
  connectivity: DashboardConnectivity | null,
  t: ReturnType<typeof useTranslation>["t"],
): ConnectivityPresentation {
  if (!connectivity) {
    return {
      summary: t("dashboard.connectivity.loading"),
      recommendation: t("dashboard.connectivity.noRecommendation"),
      probe: t("dashboard.values.notSet"),
    };
  }

  let summary = t("dashboard.connectivity.healthySummary");
  let recommendation = t("dashboard.connectivity.noRecommendation");

  if (connectivity.fakeIpDetected) {
    summary = t("dashboard.connectivity.fakeIpSummary");
    recommendation = t("dashboard.connectivity.directAndFilter");
  } else if (connectivity.probeStatus !== "reachable") {
    summary = t("dashboard.connectivity.transportSummary");
    recommendation = t("dashboard.connectivity.checkRoute");
  } else if (connectivity.proxyEnvDetected) {
    summary = t("dashboard.connectivity.proxyEnvSummary");
    recommendation = t("dashboard.connectivity.verifyProviderRoute");
  }

  if (connectivity.recommendation === "direct_host_and_fake_ip_filter") {
    recommendation = t("dashboard.connectivity.directAndFilter");
  } else if (connectivity.recommendation === "check_network_route") {
    recommendation = t("dashboard.connectivity.checkRoute");
  }

  const probe =
    connectivity.probeStatus === "reachable"
      ? t("dashboard.connectivity.probeReachable", {
          code: connectivity.probeStatusCode ?? "-",
        })
      : t("dashboard.connectivity.probeTransportFailure");

  return { summary, recommendation, probe };
}

export default function DashboardPage() {
  const { t } = useTranslation();
  const {
    canAccessProtectedApi,
    acceptValidatedOnboardingStatus,
    authRevision,
    markUnauthorized,
    status,
    authMode,
    tokenPath,
    tokenEnv,
  } =
    useWebConnection();
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [providers, setProviders] = useState<DashboardProviderItem[]>([]);
  const [runtime, setRuntime] = useState<DashboardRuntime | null>(null);
  const [connectivity, setConnectivity] = useState<DashboardConnectivity | null>(null);
  const [config, setConfig] = useState<DashboardConfigSnapshot | null>(null);
  const [tools, setTools] = useState<DashboardTools | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [providerKind, setProviderKind] = useState("");
  const [providerModel, setProviderModel] = useState("");
  const [providerRoute, setProviderRoute] = useState("");
  const [providerApiKey, setProviderApiKey] = useState("");
  const [providerApiKeyDirty, setProviderApiKeyDirty] = useState(false);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsNotice, setSettingsNotice] = useState<string | null>(null);
  const [isSavingSettings, setIsSavingSettings] = useState(false);
  const [isRefreshingDiagnostics, setIsRefreshingDiagnostics] = useState(false);
  const [settingsModal, setSettingsModal] = useState<SettingsModalState | null>(null);
  const [showDebugConsole, setShowDebugConsole] = useState(false);
  const [debugConsole, setDebugConsole] = useState<DashboardDebugConsole | null>(null);
  const [debugConsoleError, setDebugConsoleError] = useState<string | null>(null);

  async function reloadDashboardData() {
    setError(null);
    const [
      loadedSummary,
      loadedProviders,
      loadedRuntime,
      loadedConnectivity,
      loadedConfig,
      loadedTools,
    ] = await Promise.all([
      dashboardApi.loadSummary(),
      dashboardApi.loadProviders(),
      dashboardApi.loadRuntime(),
      dashboardApi.loadConnectivity(),
      dashboardApi.loadConfig(),
      dashboardApi.loadTools(),
    ]);

    setSummary(loadedSummary);
    setProviders(loadedProviders.items);
    setRuntime(loadedRuntime);
    setConnectivity(loadedConnectivity);
    setConfig(loadedConfig);
    setTools(loadedTools);
  }

  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      setSummary(null);
      setProviders([]);
      setRuntime(null);
      setConnectivity(null);
      setConfig(null);
      setTools(null);
      setError(
        status === "unauthorized"
          ? authMode === "same_origin_session"
            ? t("auth.sessionInvalidBody")
            : t("auth.invalidBody", {
                tokenPath: tokenPath ?? "",
                tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN",
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
    };
  }, [authRevision, canAccessProtectedApi, markUnauthorized, status, t]);

  const activeProvider =
    providers.find((provider) => provider.enabled) ?? providers[0] ?? null;
  const runtimeTone: Tone =
    runtime?.status === "ready" ? "good" : runtime?.status ? "warn" : "muted";
  const providerTone: Tone = activeProvider?.enabled ? "good" : "muted";
  const apiKeyState = config?.apiKeyConfigured
    ? t("dashboard.values.configured")
    : t("dashboard.values.missing");
  const enabledTools = tools?.items.filter((item) => item.enabled).length ?? 0;
  const approvalDisplay = formatApprovalMode(tools?.approvalMode, t);
  const shellPolicyDisplay = formatShellPolicy(tools?.shellDefaultMode, t);
  const promptModeDisplay = formatPromptMode(config?.promptMode, t);
  const personalityDisplay = formatPersonality(config?.personality, t);
  const memoryProfileDisplay = formatMemoryProfile(config?.memoryProfile, t);
  const connectivityCopy = buildConnectivityCopy(connectivity, t);

  const summaryCards: SummaryCard[] = [
    {
      key: "runtime",
      value: runtime?.status ?? "Loading",
      chip: runtime?.source ?? t("dashboard.values.live"),
      tone: runtimeTone,
      items: [
        {
          label: t("dashboard.fields.configPath"),
          value: runtime?.configPath ?? t("dashboard.values.notSet"),
        },
        {
          label: t("dashboard.fields.ingest"),
          value: runtime?.ingestMode ?? t("dashboard.values.notSet"),
        },
      ],
    },
    {
      key: "memory",
      value: summary?.memoryBackend ?? t("dashboard.values.none"),
      chip: config?.memoryProfile ?? t("dashboard.values.stored"),
      tone: "muted",
      items: [
        {
          label: t("dashboard.fields.sessions"),
          value: String(summary?.sessionCount ?? 0),
        },
        {
          label: t("dashboard.fields.memoryMode"),
          value: runtime?.memoryMode ?? t("dashboard.values.notSet"),
        },
      ],
    },
    {
      key: "install",
      value: summary?.webInstallMode ?? t("dashboard.values.none"),
      chip: t("dashboard.values.optional"),
      tone: "muted",
      items: [
        { label: t("dashboard.fields.surface"), value: t("dashboard.values.webConsole") },
        { label: t("dashboard.fields.hosting"), value: t("dashboard.values.localOnly") },
      ],
    },
    {
      key: "tools",
      value: `${enabledTools}/${tools?.items.length ?? 0}`,
      chip: approvalDisplay,
      tone: enabledTools > 0 ? "good" : "muted",
      items: [
        {
          label: t("dashboard.fields.approval"),
          value: approvalDisplay,
        },
        {
          label: t("dashboard.fields.shellPolicy"),
          value: shellPolicyDisplay,
        },
      ],
    },
  ];

  const toolItems: DashboardToolItem[] = tools?.items ?? [];
  const debugConsoleBlocks = debugConsole?.blocks ?? [
    {
      id: "loading",
      kind: "loading",
      startedAt: "",
      header: "Loading runtime and log output...",
      lines: [],
    },
  ];
  const debugConsoleCommand =
    debugConsole?.command ?? "$ loongclaw web debug --readonly";

  useEffect(() => {
    setProviderKind(activeProvider?.id ?? providers[0]?.id ?? "");
    setProviderModel(config?.model ?? "");
    setProviderRoute(config?.endpoint ?? "");
    setProviderApiKey(config?.apiKeyConfigured ? config.apiKeyMasked ?? "••••••••" : "");
    setProviderApiKeyDirty(false);
    setSettingsError(null);
  }, [activeProvider?.id, config?.endpoint, config?.model, providers]);

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
      await reloadDashboardData();
      setSettingsNotice(t("dashboard.settings.refreshed"));
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
    try {
      setSettingsModal({
        phase: "pending",
        title: t("dashboard.settings.applyPending"),
        body: t("onboarding.validation.pending"),
      });
      const result = await onboardingApi.applyProvider({
        kind: providerKind,
        model: providerModel,
        baseUrlOrEndpoint: providerRoute,
        ...(providerApiKeyDirty && providerApiKey.trim()
          ? { apiKey: providerApiKey.trim() }
          : {}),
      });
      if (result.passed) {
        acceptValidatedOnboardingStatus(result.status);
        await reloadDashboardData();
        setProviderApiKey("");
        setProviderApiKeyDirty(false);
        setSettingsModal({
          phase: "success",
          title: t("dashboard.settings.saved"),
          body: t("onboarding.validation.success"),
        });
        setSettingsNotice(t("dashboard.settings.saved"));
        await wait(1100);
      } else {
        setSettingsModal({
          phase: "error",
          title: t("onboarding.validation.failed"),
          body: readProviderValidationFailure(result.credentialStatus, t),
        });
        await reloadDashboardData();
        setSettingsError(readProviderValidationFailure(result.credentialStatus, t));
        await wait(1600);
      }
    } catch (saveError) {
      if (saveError instanceof ApiRequestError) {
        if (saveError.status === 401) {
          markUnauthorized();
        }
        setSettingsError(saveError.message);
      } else {
        setSettingsError(t("dashboard.settings.saveFailed"));
      }
      setSettingsModal({
        phase: "error",
        title: t("dashboard.settings.saveFailed"),
        body:
          saveError instanceof ApiRequestError
            ? saveError.message
            : t("dashboard.settings.saveFailed"),
      });
      await wait(1600);
    } finally {
      setSettingsModal(null);
      setIsSavingSettings(false);
    }
  }

  function handleApiKeyFocus() {
    if (config?.apiKeyConfigured && !providerApiKeyDirty) {
      setProviderApiKey("");
      setProviderApiKeyDirty(true);
    }
  }

  return (
    <div className="page">
      {settingsModal ? (
        <div className="dashboard-modal-backdrop" role="status" aria-live="polite">
          <div
            className={`dashboard-modal dashboard-modal-${settingsModal.phase}`}
          >
            <div className="dashboard-modal-title">{settingsModal.title}</div>
            <p className="dashboard-modal-body">{settingsModal.body}</p>
          </div>
        </div>
      ) : null}

      <section className="hero-block">
        <div className="dashboard-hero-head">
          <div>
            <div className="hero-eyebrow">{t("dashboard.eyebrow")}</div>
            <h1 className="hero-title">
              {showDebugConsole
                ? t("dashboard.debug.title", { defaultValue: "调试控制台" })
                : t("dashboard.title")}
            </h1>
            <p className="hero-subtitle">
              {showDebugConsole
                ? t("dashboard.debug.subtitle", {
                    defaultValue: "在一个只读的终端视图里查看当前 runtime、provider、工具与配置快照。",
                  })
                : t("dashboard.subtitle")}
            </p>
          </div>

          <button
            type="button"
            className="hero-btn hero-btn-secondary dashboard-debug-toggle"
            onClick={() => setShowDebugConsole((value) => !value)}
          >
            {showDebugConsole
              ? t("dashboard.debug.back", { defaultValue: "返回 Dashboard" })
              : t("dashboard.debug.open", { defaultValue: "Debug Console" })}
          </button>
        </div>
      </section>

      {showDebugConsole ? (
        <section className="dashboard-debug-shell">
          <Panel
            className="dashboard-debug-panel"
            eyebrow={t("dashboard.debug.eyebrow", { defaultValue: "Runtime / Debug Console" })}
            title={t("dashboard.debug.panelTitle", { defaultValue: "只读调试视图" })}
          >
            <DebugConsolePanel
              command={debugConsoleCommand}
              blocks={debugConsoleBlocks}
              error={debugConsoleError}
              emptyLabel={t("dashboard.values.notSet")}
            />
          </Panel>
        </section>
      ) : (
        <>
          <section className="dashboard-summary-grid">
            {summaryCards.map((card) => (
              <article key={card.key} className="dashboard-stat-card">
                <div className="dashboard-stat-top">
                  <div className="dashboard-stat-label">{t(`dashboard.cards.${card.key}`)}</div>
                  <span className={`dashboard-pill dashboard-pill-${card.tone}`}>{card.chip}</span>
                </div>
                <div className="dashboard-stat-value">{card.value}</div>
                <div className="dashboard-stat-list">
                  {card.items.map((item) => (
                    <div key={item.label} className="dashboard-stat-row">
                      <span>{item.label}</span>
                      <strong title={item.value}>{item.value}</strong>
                    </div>
                  ))}
                </div>
              </article>
            ))}
          </section>

          {error ? <div className="empty-state dashboard-error">{error}</div> : null}

          <section className="dashboard-layout">
            <div className="dashboard-main-column">
              <Panel
                eyebrow={t("dashboard.sections.providerEyebrow")}
                title={t("dashboard.sections.providerTitle")}
                aside={
                  <span className={`dashboard-pill dashboard-pill-${providerTone}`}>
                    {activeProvider?.enabled
                      ? t("dashboard.values.active")
                      : t("dashboard.values.inactive")}
                  </span>
                }
              >
            <div className="dashboard-provider-head">
              <div>
                <div className="dashboard-provider-name">
                  {activeProvider?.label ?? t("dashboard.values.none")}
                </div>
                <div
                  className="dashboard-provider-subtitle"
                  title={config?.model ?? t("dashboard.values.noModel")}
                >
                  {config?.model ?? t("dashboard.values.noModel")}
                </div>
              </div>

              <div className="dashboard-provider-meta">
                <div className="dashboard-meta-stack">
                  <span>{t("dashboard.fields.endpoint")}</span>
                  <strong title={config?.endpoint ?? t("dashboard.values.notSet")}>
                    {config?.endpoint ?? t("dashboard.values.notSet")}
                  </strong>
                </div>
                <div className="dashboard-meta-stack">
                  <span>{t("dashboard.fields.apiKey")}</span>
                  <strong>{apiKeyState}</strong>
                </div>
              </div>
            </div>

            <div className="dashboard-kv-grid">
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.providerId")}</span>
                <strong>{activeProvider?.id ?? t("dashboard.values.none")}</strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.defaultRole")}</span>
                <strong>
                  {activeProvider?.defaultForKind
                    ? t("dashboard.values.default")
                    : t("dashboard.values.secondary")}
                </strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.lastProvider")}</span>
                <strong>{config?.lastProvider ?? t("dashboard.values.none")}</strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.memory")}</span>
                <strong>{summary?.memoryBackend ?? t("dashboard.values.none")}</strong>
              </div>
            </div>

            <div className="dashboard-provider-list">
              {providers.length > 0 ? (
                providers.map((provider) => (
                  <div key={provider.id} className="dashboard-provider-item">
                    <div>
                      <div className="dashboard-provider-item-title">{provider.label}</div>
                      <div className="dashboard-provider-item-meta" title={provider.model}>
                        {provider.model}
                      </div>
                    </div>
                    <span
                      className={`dashboard-pill dashboard-pill-${provider.enabled ? "good" : "muted"}`}
                    >
                      {provider.enabled
                        ? t("dashboard.values.active")
                        : t("dashboard.values.standby")}
                    </span>
                  </div>
                ))
              ) : (
                <div className="empty-state">{t("dashboard.values.noProviders")}</div>
              )}
            </div>
              </Panel>

              <section className="dashboard-settings">
                <Panel
                  eyebrow={t("dashboard.settings.eyebrow")}
                  title={t("dashboard.settings.title")}
                >
              <div className="settings-header">
                <p className="panel-copy">{t("dashboard.settings.subtitle")}</p>
              </div>
              <form className="settings-form" onSubmit={handleApplySettings}>
                <label className="settings-field">
                  <span className="settings-label">{t("dashboard.settings.activeProvider")}</span>
                  <select
                    className="settings-input"
                    value={providerKind}
                    onChange={(event) => {
                      const nextKind = event.target.value;
                      setProviderKind(nextKind);
                      if (providerRoute === (config?.endpoint ?? "")) {
                        // Clear the inherited route when switching kinds so save can
                        // fall back to the selected provider's default endpoint.
                        setProviderRoute("");
                      }
                    }}
                  >
                    {providers.map((provider) => (
                      <option key={provider.id} value={provider.id}>
                        {provider.label}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="settings-field">
                  <span className="settings-label">{t("dashboard.settings.model")}</span>
                  <input
                    className="settings-input"
                    value={providerModel}
                    onChange={(event) => setProviderModel(event.target.value)}
                  />
                </label>

                <label className="settings-field">
                  <span className="settings-label">{t("dashboard.settings.endpoint")}</span>
                  <input
                    className="settings-input"
                    value={providerRoute}
                    onChange={(event) => setProviderRoute(event.target.value)}
                  />
                </label>

                <label className="settings-field">
                  <span className="settings-label">{t("dashboard.settings.apiKey")}</span>
                  <input
                    className="settings-input"
                    type="password"
                    autoComplete="off"
                    value={providerApiKey}
                    onFocus={handleApiKeyFocus}
                    onChange={(event) => {
                      setProviderApiKey(event.target.value);
                      setProviderApiKeyDirty(true);
                    }}
                    placeholder={
                      config?.apiKeyConfigured
                        ? ""
                        : t("dashboard.settings.apiKeyPlaceholder")
                    }
                  />
                  <span className="settings-helper">
                    {config?.apiKeyConfigured
                      ? t("dashboard.settings.apiKeyMasked")
                      : t("dashboard.settings.apiKeyHelper")}
                  </span>
                </label>

                {settingsError ? (
                  <p className="settings-note dashboard-error">{settingsError}</p>
                ) : null}
                {settingsNotice ? <p className="settings-note">{settingsNotice}</p> : null}

                <div className="settings-actions">
                  <button
                    type="button"
                    className="hero-btn hero-btn-secondary"
                    onClick={handleRefreshDiagnostics}
                    disabled={isRefreshingDiagnostics || isSavingSettings}
                  >
                    {isRefreshingDiagnostics
                      ? t("dashboard.settings.validatePending")
                      : t("dashboard.settings.validate")}
                  </button>
                  <button
                    type="submit"
                    className="hero-btn hero-btn-primary"
                    disabled={isSavingSettings || isRefreshingDiagnostics}
                  >
                    {isSavingSettings
                      ? t("dashboard.settings.applyPending")
                      : t("dashboard.settings.apply")}
                  </button>
                </div>

                <p className="settings-note">{t("dashboard.settings.helper")}</p>
              </form>
                </Panel>
              </section>
            </div>

            <div className="dashboard-side-column">
              <Panel
                eyebrow={t("dashboard.sections.connectivityEyebrow")}
                title={t("dashboard.sections.connectivityTitle")}
              >
            <p className="panel-copy">{connectivityCopy.summary}</p>
            <div className="dashboard-kv-list">
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.providerHost")}</span>
                <strong title={connectivity?.host ?? t("dashboard.values.notSet")}>
                  {connectivity?.host ?? t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.dns")}</span>
                <strong
                  title={
                    connectivity?.dnsAddresses.length
                      ? connectivity.dnsAddresses.join(", ")
                      : t("dashboard.values.notSet")
                  }
                >
                  {connectivity?.dnsAddresses.length
                    ? connectivity.dnsAddresses.join(", ")
                    : t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.probe")}</span>
                <strong title={connectivityCopy.probe}>{connectivityCopy.probe}</strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.routing")}</span>
                <strong title={connectivityCopy.recommendation}>
                  {connectivityCopy.recommendation}
                </strong>
              </div>
            </div>
              </Panel>

              <Panel
                eyebrow={t("dashboard.sections.runtimeEyebrow")}
                title={t("dashboard.sections.runtimeTitle")}
              >
            <div className="dashboard-stacked-section">
              <div className="dashboard-section-heading">
                {t("dashboard.sections.runtimeDetailLabel")}
              </div>
              <div className="dashboard-kv-list">
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.runtime")}</span>
                  <strong title={runtime?.status ?? "Loading"}>
                    {runtime?.status ?? "Loading"}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.source")}</span>
                  <strong title={runtime?.source ?? t("dashboard.values.notSet")}>
                    {runtime?.source ?? t("dashboard.values.notSet")}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.ingest")}</span>
                  <strong title={runtime?.ingestMode ?? t("dashboard.values.notSet")}>
                    {runtime?.ingestMode ?? t("dashboard.values.notSet")}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.acp")}</span>
                  <strong>
                    {runtime?.acpEnabled
                      ? t("dashboard.values.enabled")
                      : t("dashboard.values.disabled")}
                  </strong>
                </div>
              </div>
            </div>

            <div className="dashboard-stacked-section dashboard-stacked-section-separated">
              <div className="dashboard-section-heading">
                {t("dashboard.sections.configDetailLabel")}
              </div>
              <div className="dashboard-kv-list">
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.apiKey")}</span>
                  <strong title={apiKeyState}>{apiKeyState}</strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.memoryProfile")}</span>
                  <strong title={memoryProfileDisplay}>
                    {memoryProfileDisplay}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.personality")}</span>
                  <strong title={personalityDisplay}>{personalityDisplay}</strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.promptMode")}</span>
                  <strong title={promptModeDisplay}>{promptModeDisplay}</strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.promptAddendum")}</span>
                  <strong>
                    {config?.promptAddendumConfigured
                      ? t("dashboard.values.configured")
                      : t("dashboard.values.missing")}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.sqlitePath")}</span>
                  <strong title={config?.sqlitePath ?? t("dashboard.values.notSet")}>
                    {config?.sqlitePath ?? t("dashboard.values.notSet")}
                  </strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.fileRoot")}</span>
                  <strong title={config?.fileRoot ?? t("dashboard.values.notSet")}>
                    {config?.fileRoot ?? t("dashboard.values.notSet")}
                  </strong>
                </div>
              </div>
            </div>
              </Panel>

              <Panel
                eyebrow={t("dashboard.sections.toolsEyebrow")}
                title={t("dashboard.sections.toolsTitle")}
              >
            <div className="dashboard-tool-summary">
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.approval")}</span>
                <strong title={approvalDisplay}>
                  {approvalDisplay}
                </strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.allowed")}</span>
                <strong title={String(tools?.shellAllowCount ?? 0)}>
                  {tools?.shellAllowCount ?? 0}
                </strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.denied")}</span>
                <strong title={String(tools?.shellDenyCount ?? 0)}>
                  {tools?.shellDenyCount ?? 0}
                </strong>
              </div>
              <div className="dashboard-kv-card">
                <span>{t("dashboard.fields.shellPolicy")}</span>
                <strong title={shellPolicyDisplay}>{shellPolicyDisplay}</strong>
              </div>
            </div>

            <div className="dashboard-tool-grid">
              {toolItems.map((item) => (
                <article key={item.id} className="dashboard-tool-card">
                  <div className="dashboard-tool-card-top">
                    <div className="dashboard-tool-card-title">
                      {t(`dashboard.toolItems.${item.id}`)}
                    </div>
                    <span
                      className={`dashboard-pill dashboard-pill-${item.enabled ? "good" : "muted"} dashboard-pill-compact`}
                    >
                      {item.enabled ? t("dashboard.values.enabled") : t("dashboard.values.disabled")}
                    </span>
                  </div>
                  <div className="dashboard-tool-card-meta" title={item.detail}>
                    {item.detail}
                  </div>
                </article>
              ))}
            </div>
              </Panel>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
