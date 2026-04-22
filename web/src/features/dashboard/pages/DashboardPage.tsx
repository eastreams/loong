import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw } from "lucide-react";
import { ChoiceField } from "../../../components/inputs/ChoiceField";
import { Panel } from "../../../components/surfaces/Panel";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { useDashboardData } from "../hooks/useDashboardData";
import {
  MEMORY_PROFILE_OPTIONS,
  PERSONALITY_OPTIONS,
  normalizePersonalityForUi,
  usePreferencesForm,
  useProviderConfigForm,
} from "../../onboarding/provider/providerConfig";
import { DebugConsolePanel } from "../components/DebugConsolePanel";
import {
  buildProviderKindOptions,
  type ProviderCatalogItem,
} from "../../onboarding/provider/providerCatalog";
import {
  dashboardApi,
  type DashboardConnectivity,
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

// Removed SummaryCard interface

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
    case "medium_balanced":
      return t("dashboard.values.approvalMediumBalanced");
    case "strict":
      return t("dashboard.values.approvalStrict");
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

function formatAutonomyProfile(
  autonomyProfile: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!autonomyProfile) {
    return t("dashboard.values.notSet");
  }

  switch (autonomyProfile) {
    case "discovery_only":
      return t("dashboard.values.autonomyDiscoveryOnly");
    case "guided_acquisition":
      return t("dashboard.values.autonomyGuidedAcquisition");
    case "bounded_autonomous":
      return t("dashboard.values.autonomyBoundedAutonomous");
    default:
      return autonomyProfile;
  }
}

function formatConsentDefaultMode(
  consentDefaultMode: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!consentDefaultMode) {
    return t("dashboard.values.notSet");
  }

  switch (consentDefaultMode) {
    case "prompt":
      return t("dashboard.values.consentPrompt");
    case "auto":
      return t("dashboard.values.consentAuto");
    case "full":
      return t("dashboard.values.consentFull");
    default:
      return consentDefaultMode;
  }
}

function formatPersonality(
  personality: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!personality) {
    return t("dashboard.values.notSet");
  }

  switch (normalizePersonalityForUi(personality)) {
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

function formatToolSource(
  source: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (source) {
    case "native":
      return t("dashboard.toolMeta.native");
    case "companion":
      return t("dashboard.toolMeta.companion");
    case "provider":
      return t("dashboard.toolMeta.provider");
    case "local":
      return t("dashboard.toolMeta.local");
    case "catalog":
      return t("dashboard.toolMeta.catalog");
    default:
      return source;
  }
}

function formatToolCapabilityState(
  state: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (state) {
    case "discoverable":
      return t("dashboard.toolMeta.discoverable");
    case "executable":
      return t("dashboard.toolMeta.executable");
    case "policy_limited":
      return t("dashboard.toolMeta.policyLimited");
    case "runtime_unavailable":
      return t("dashboard.toolMeta.runtimeUnavailable");
    default:
      return state;
  }
}

function formatApprovalStatus(
  status: string | null | undefined,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (!status) {
    return t("dashboard.values.notSet");
  }

  switch (status) {
    case "pending":
      return t("dashboard.approvals.status.pending");
    case "approved":
      return t("dashboard.approvals.status.approved");
    case "executing":
      return t("dashboard.approvals.status.executing");
    case "executed":
      return t("dashboard.approvals.status.executed");
    case "denied":
      return t("dashboard.approvals.status.denied");
    case "expired":
      return t("dashboard.approvals.status.expired");
    case "cancelled":
      return t("dashboard.approvals.status.cancelled");
    default:
      return status;
  }
}

function approvalTone(status: string | null | undefined): Tone {
  switch (status) {
    case "pending":
      return "warn";
    case "approved":
    case "executing":
      return "good";
    default:
      return "muted";
  }
}

function formatToolLabel(
  id: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (id) {
    case "shell_policy":
    case "bash_exec":
      return t("dashboard.toolItems.bash_exec", { defaultValue: "Bash execution" });
    default:
      return t(`dashboard.toolItems.${id}`, {
        defaultValue: id.replace(/_/g, " "),
      });
  }
}

function buildToolMeta(
  item: DashboardToolItem,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  const segments = [
    formatToolSource(item.source, t),
    formatToolCapabilityState(item.capabilityState, t),
    item.detail,
  ].filter((part) => part && part.trim().length > 0);

  return segments.join(" · ");
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
  const connection = useWebConnection();
  const { canAccessProtectedApi } = connection;
  const [providerCatalog, setProviderCatalog] = useState<ProviderCatalogItem[]>([]);
  const [providerCatalogLoadFailed, setProviderCatalogLoadFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;

    if (!canAccessProtectedApi) {
      setProviderCatalog([]);
      setProviderCatalogLoadFailed(false);
      return () => {
        cancelled = true;
      };
    }

    void dashboardApi
      .loadProviderCatalog()
      .then((items) => {
        if (!cancelled) {
          setProviderCatalog(items);
          setProviderCatalogLoadFailed(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setProviderCatalog([]);
          setProviderCatalogLoadFailed(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [canAccessProtectedApi]);

  // These forms depend on initial data but manage their own state
  const providerForm = useProviderConfigForm({
    kind: "",
    model: "",
    baseUrlOrEndpoint: "",
    apiKeyConfigured: false,
  }, providerCatalog);

  const preferencesForm = usePreferencesForm({
    personality: "calm_engineering",
    memoryProfile: "window_only",
    slidingWindow: 12,
    promptAddendum: "",
  });

  const { state, actions } = useDashboardData({
    t,
    connection,
    providerForm,
    preferencesForm,
  });

  const {
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
  } = state;

  const { setShowDebugConsole, handleRefreshDiagnostics, handleApplySettings } =
    actions;

  const activeProvider =
    providers.find((provider) => provider.enabled) ?? providers[0] ?? null;
  const providerTone: Tone = activeProvider?.enabled ? "good" : "muted";
  const apiKeyState = config?.apiKeyConfigured
    ? t("dashboard.values.configured")
    : t("dashboard.values.missing");
  const enabledTools = tools?.items.filter((item) => item.enabled).length ?? 0;
  const approvalDisplay = formatApprovalMode(tools?.approvalMode, t);
  const autonomyProfileDisplay = formatAutonomyProfile(tools?.autonomyProfile, t);
  const consentDefaultModeDisplay = formatConsentDefaultMode(
    tools?.consentDefaultMode,
    t,
  );
  const shellPolicyDisplay = formatShellPolicy(tools?.shellDefaultMode, t);
  const promptModeDisplay = formatPromptMode(config?.promptMode, t);
  const personalityDisplay = formatPersonality(config?.personality, t);
  const memoryProfileDisplay = formatMemoryProfile(config?.memoryProfile, t);
  const connectivityCopy = buildConnectivityCopy(connectivity, t);

  const toolItems: DashboardToolItem[] = tools?.items ?? [];
  const approvalItems = approvals?.items ?? [];
  const providerKindOptions = buildProviderKindOptions(
    providerCatalog,
    providerForm.kind,
  );
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

  return (
    <div className="page page-dashboard">
      <div className="dashboard-shell">
        <div className="dashboard-sidebar">
          <Panel
            title={t("dashboard.sections.providerTitle")}
          >
            <div className="dashboard-sidebar-provider-head">
              <div className="dashboard-sidebar-provider-name-row">
                <div className="dashboard-sidebar-provider-name">
                  {activeProvider?.label ?? t("dashboard.values.none")}
                </div>
                <span className={`dashboard-pill dashboard-pill-${providerTone} dashboard-pill-compact`}>
                  {activeProvider?.enabled
                    ? t("dashboard.values.active")
                    : t("dashboard.values.inactive")}
                </span>
              </div>
              <div className="dashboard-sidebar-provider-model" title={config?.model ?? t("dashboard.values.noModel")}>
                {config?.model ?? t("dashboard.values.noModel")}
              </div>
            </div>

            <div className="dashboard-kv-list">
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.endpoint")}</span>
                <strong title={config?.endpoint ?? t("dashboard.values.notSet")}>
                  {config?.endpoint ?? t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.apiKey")}</span>
                <strong>{apiKeyState}</strong>
              </div>
            </div>

            <div className="dashboard-sidebar-divider" />

            <div className="dashboard-stacked-section">
              <div className="dashboard-section-heading">
                {t("dashboard.sections.connectivityDetailLabel")}
                <button
                  className="dashboard-refresh-btn"
                  onClick={handleRefreshDiagnostics}
                  disabled={isRefreshingDiagnostics}
                  aria-label={t("dashboard.actions.refreshDiagnostics")}
                  title={t("dashboard.actions.refreshDiagnostics")}
                >
                  <RefreshCw className={isRefreshingDiagnostics ? "animate-spin" : ""} size={14} />
                </button>
              </div>
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
            </div>

            <div className="dashboard-sidebar-divider" />

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

            <div className="dashboard-sidebar-divider" />

            <div className="dashboard-stacked-section">
              <div className="dashboard-section-heading">
                {t("dashboard.sections.configDetailLabel")}
              </div>
              <div className="dashboard-kv-list">
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.memoryProfile")}</span>
                  <strong title={memoryProfileDisplay}>{memoryProfileDisplay}</strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.fields.slidingWindow")}</span>
                  <strong>
                    {config?.slidingWindow ?? t("dashboard.values.notSet")}
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
        </div>
        <div className="dashboard-center">
          <div className="dashboard-center-inner">
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
                      ? t("dashboard.debug.title", { defaultValue: "调试终端" })
                      : t("dashboard.title")}
                  </h1>
                  <p className="hero-subtitle">
                    {showDebugConsole
                      ? t("dashboard.debug.subtitle", {
                        defaultValue:
                          "以只读终端视图查看当前轮次、工具活动与进程输出。",
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
                    ? t("dashboard.debug.back", { defaultValue: "返回状态" })
                    : t("dashboard.debug.open", { defaultValue: "Debug Terminal" })}
                </button>
              </div>
            </section>

            {showDebugConsole ? (
              <section className="dashboard-debug-shell">
                <Panel
                  className="dashboard-debug-panel"
                  eyebrow={t("dashboard.debug.eyebrow", { defaultValue: "Runtime / Debug Terminal" })}
                  title=""
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
                {/* Dashboard summary grid removed as metrics are now natively tracked over sidebars */}

                {error ? <div className="empty-state dashboard-error">{error}</div> : null}

                <section className="dashboard-settings">
                  <Panel
                    className="dashboard-settings-panel"
                    title={t("dashboard.settings.title")}
                  >

                    <form className="settings-form" onSubmit={handleApplySettings}>
                      <ChoiceField
                        id="dashboard-provider-kind"
                        label={t("dashboard.settings.activeProvider")}
                        value={providerForm.kind}
                        options={providerKindOptions}
                        onSelect={(val) => providerForm.setKindWithRouteReset(val)}
                      />
                      {providerCatalogLoadFailed ? (
                        <p className="settings-note dashboard-error">
                          {t("dashboard.settings.catalogLoadFailed")}
                        </p>
                      ) : null}

                      <label className="settings-field">
                        <span className="settings-label">{t("dashboard.settings.model")}</span>
                        <input
                          className="settings-input"
                          value={providerForm.model}
                          onChange={(event) => providerForm.setModel(event.target.value)}
                        />
                      </label>

                      <label className="settings-field">
                        <span className="settings-label">{t("dashboard.settings.endpoint")}</span>
                        <input
                          className="settings-input"
                          value={providerForm.baseUrlOrEndpoint}
                          onChange={(event) =>
                            providerForm.setBaseUrlOrEndpoint(event.target.value)
                          }
                        />
                      </label>

                      <label className="settings-field">
                        <span className="settings-label">{t("dashboard.settings.apiKey")}</span>
                        <input
                          className="settings-input"
                          type="password"
                          autoComplete="off"
                          value={providerForm.apiKey}
                          onFocus={providerForm.handleApiKeyFocus}
                          onChange={(event) => {
                            providerForm.setApiKeyValue(event.target.value);
                          }}
                          placeholder={
                            config?.apiKeyConfigured
                              ? t("dashboard.settings.apiKeyPlaceholderConfigured")
                              : t("dashboard.settings.apiKeyPlaceholder")
                          }
                        />
                      </label>

                      <ChoiceField
                        id="dashboard-preferences-personality"
                        label={t("onboarding.preferences.personality")}
                        value={preferencesForm.personality}
                        options={PERSONALITY_OPTIONS.map((item) => ({
                          value: item,
                          label:
                            item === "calm_engineering"
                              ? t("onboarding.preferences.personalityCalmEngineering")
                              : item === "friendly_collab"
                                ? t("onboarding.preferences.personalityFriendlyCollab")
                                : t("onboarding.preferences.personalityAutonomousExecutor"),
                        }))}
                        onSelect={(val) => preferencesForm.setPersonality(val)}
                      />

                      <ChoiceField
                        id="dashboard-preferences-memory-profile"
                        label={t("onboarding.preferences.memoryProfile")}
                        value={preferencesForm.memoryProfile}
                        options={MEMORY_PROFILE_OPTIONS.map((item) => ({
                          value: item,
                          label:
                            item === "window_only"
                              ? t("onboarding.preferences.memoryProfileWindowOnly")
                              : item === "window_plus_summary"
                                ? t("onboarding.preferences.memoryProfileWindowPlusSummary")
                                : t("onboarding.preferences.memoryProfileProfilePlusWindow"),
                        }))}
                        onSelect={(val) => preferencesForm.setMemoryProfile(val)}
                      />

                      <label className="settings-field">
                        <span className="settings-label">
                          {t("dashboard.settings.slidingWindow")}
                        </span>
                        <input
                          className="settings-input"
                          type="text"
                          inputMode="numeric"
                          value={preferencesForm.slidingWindow}
                          onChange={(event) =>
                            preferencesForm.setSlidingWindow(event.target.value)
                          }
                          placeholder="24"
                        />
                      </label>

                      <label className="settings-field">
                        <span className="settings-label">{t("onboarding.preferences.promptAddendum")}</span>
                        <textarea
                          className="settings-input settings-textarea"
                          value={preferencesForm.promptAddendum}
                          onChange={(event) => preferencesForm.setPromptAddendum(event.target.value)}
                          placeholder={t("onboarding.preferences.promptAddendumPlaceholder")}
                        />
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

              </>
            )}
          </div>
        </div>
        <div className="dashboard-sidebar">
          <Panel
            title={t("dashboard.sections.toolsTitle")}
          >
            <div className="dashboard-section-heading">
              {t("dashboard.sections.toolsGovernanceLabel")}
            </div>
            <div className="dashboard-tool-stats">
              <div className="dashboard-tool-stat">
                <span className="dashboard-tool-stat-label">{t("dashboard.fields.approval")}</span>
                <span className="dashboard-tool-stat-value">{approvalDisplay}</span>
              </div>
              <div className="dashboard-tool-stat">
                <span className="dashboard-tool-stat-label">{t("dashboard.fields.autonomy")}</span>
                <span className="dashboard-tool-stat-value" title={autonomyProfileDisplay}>
                  {autonomyProfileDisplay}
                </span>
              </div>
              <div className="dashboard-tool-stat">
                <span className="dashboard-tool-stat-label">{t("dashboard.fields.consent")}</span>
                <span className="dashboard-tool-stat-value" title={consentDefaultModeDisplay}>
                  {consentDefaultModeDisplay}
                </span>
              </div>
              <div className="dashboard-tool-stat">
                <span className="dashboard-tool-stat-label">{t("dashboard.fields.shellPolicy")}</span>
                <span className="dashboard-tool-stat-value" title={shellPolicyDisplay}>{shellPolicyDisplay}</span>
              </div>
            </div>

            <div className="dashboard-kv-list">
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.sessionMutation")}</span>
                <strong>
                  {tools
                    ? tools.sessionsAllowMutation
                      ? t("dashboard.values.enabled")
                      : t("dashboard.values.disabled")
                    : t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.externalSkillsDownloadApproval")}</span>
                <strong>
                  {tools
                    ? tools.externalSkillsRequireDownloadApproval
                      ? t("dashboard.values.yes")
                      : t("dashboard.values.no")
                    : t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.externalSkillsAutoExpose")}</span>
                <strong>
                  {tools
                    ? tools.externalSkillsAutoExposeInstalled
                      ? t("dashboard.values.enabled")
                      : t("dashboard.values.disabled")
                    : t("dashboard.values.notSet")}
                </strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.externalSkillsBlockedDomains")}</span>
                <strong>{tools?.externalSkillsBlockedDomainCount ?? 0}</strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.allowed")}</span>
                <strong>{tools?.shellAllowCount ?? 0}</strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.denied")}</span>
                <strong>{tools?.shellDenyCount ?? 0}</strong>
              </div>
              <div className="dashboard-kv-row">
                <span>{t("dashboard.fields.enabledTools")}</span>
                <strong>{enabledTools}</strong>
              </div>
            </div>

            <div className="dashboard-sidebar-divider" />

            <div className="dashboard-stacked-section">
              <div className="dashboard-section-heading">
                {t("dashboard.sections.approvalsLabel")}
              </div>
              <div className="dashboard-kv-list">
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.approvals.pendingCount")}</span>
                  <strong>{approvals?.pendingApprovalCount ?? 0}</strong>
                </div>
                <div className="dashboard-kv-row">
                  <span>{t("dashboard.approvals.activeCount")}</span>
                  <strong>{approvals?.activeApprovalCount ?? 0}</strong>
                </div>
              </div>
              {approvalItems.length > 0 ? (
                <div className="dashboard-approval-list">
                  {approvalItems.map((item) => (
                    <div key={item.approvalRequestId} className="dashboard-approval-row">
                      <div className="dashboard-approval-row-head">
                        <div className="dashboard-approval-row-name">
                          {item.visibleToolName}
                        </div>
                        <span
                          className={`dashboard-pill dashboard-pill-${approvalTone(item.status)} dashboard-pill-compact`}
                        >
                          {formatApprovalStatus(item.status, t)}
                        </span>
                      </div>
                      <div
                        className="dashboard-approval-row-session"
                        title={item.sessionTitle}
                      >
                        {item.sessionTitle}
                      </div>
                      <div
                        className="dashboard-approval-row-meta"
                        title={item.requestSummary}
                      >
                        {item.requestSummary}
                      </div>
                      <div
                        className="dashboard-approval-row-time"
                        title={item.requestedAt}
                      >
                        {t("dashboard.approvals.requestedAt", {
                          at: item.requestedAt,
                        })}
                      </div>
                      {item.lastError ? (
                        <div
                          className="dashboard-approval-row-error"
                          title={item.lastError}
                        >
                          {item.lastError}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : (
                <p className="dashboard-approval-empty">
                  {t("dashboard.approvals.empty")}
                </p>
              )}
            </div>

            <div className="dashboard-sidebar-divider" />

            <div className="dashboard-tool-list">
              {toolItems.map((item) => (
                <div key={item.id} className="dashboard-tool-row" title={item.detail}>
                  <div className="dashboard-tool-row-head">
                    <div className="dashboard-tool-row-name">
                      {formatToolLabel(item.id, t)}
                    </div>
                    <span
                      className={`dashboard-pill dashboard-pill-${item.enabled ? "good" : "muted"} dashboard-pill-compact`}
                    >
                      {item.enabled ? t("dashboard.values.enabled") : t("dashboard.values.disabled")}
                    </span>
                  </div>

                  <div className="dashboard-tool-row-meta">
                    {buildToolMeta(item, t)}
                  </div>
                </div>
              ))}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}
