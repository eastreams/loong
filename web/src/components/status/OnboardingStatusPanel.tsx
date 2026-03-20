import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { Panel } from "../surfaces/Panel";
import { useWebConnection } from "../../hooks/useWebConnection";
import { ApiRequestError } from "../../lib/api/client";
import { onboardingApi } from "../../features/onboarding/api";

function readStageCopy(
  stage: string,
  t: ReturnType<typeof useTranslation>["t"],
) {
  switch (stage) {
    case "runtime_offline":
      return {
        title: t("onboarding.stages.runtimeOffline.title"),
        body: t("onboarding.stages.runtimeOffline.body"),
      };
    case "token_pairing":
      return {
        title: t("onboarding.stages.tokenPairing.title"),
        body: t("onboarding.stages.tokenPairing.body"),
      };
    case "missing_config":
      return {
        title: t("onboarding.stages.missingConfig.title"),
        body: t("onboarding.stages.missingConfig.body"),
      };
    case "config_invalid":
      return {
        title: t("onboarding.stages.configInvalid.title"),
        body: t("onboarding.stages.configInvalid.body"),
      };
    case "provider_setup":
      return {
        title: t("onboarding.stages.providerSetup.title"),
        body: t("onboarding.stages.providerSetup.body"),
      };
    case "provider_unreachable":
      return {
        title: t("onboarding.stages.providerUnreachable.title"),
        body: t("onboarding.stages.providerUnreachable.body"),
      };
    case "ready":
      return {
        title: t("onboarding.stages.ready.title"),
        body: t("onboarding.stages.ready.body"),
      };
    default:
      return {
        title: t("onboarding.loadingTitle"),
        body: t("onboarding.loadingBody"),
      };
  }
}

export function OnboardingStatusPanel() {
  const { t } = useTranslation();
  const {
    status,
    authRequired,
    tokenEnv,
    tokenPath,
    saveToken,
    clearToken,
    onboardingLoading,
    onboardingStatus,
    onboardingValidationSatisfied,
    acknowledgeOnboarding,
    markOnboardingValidated,
    clearOnboardingValidation,
    refreshOnboardingStatus,
    autoPairingInProgress,
  } = useWebConnection();
  const [kind, setKind] = useState("");
  const [model, setModel] = useState("");
  const [baseUrlOrEndpoint, setBaseUrlOrEndpoint] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [validationMessage, setValidationMessage] = useState<string | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [tokenInput, setTokenInput] = useState("");
  const [showOptionalSettings, setShowOptionalSettings] = useState(false);
  const [personality, setPersonality] = useState("calm_engineering");
  const [memoryProfile, setMemoryProfile] = useState("window_only");
  const [promptAddendum, setPromptAddendum] = useState("");
  const [preferencesError, setPreferencesError] = useState<string | null>(null);
  const [preferencesNotice, setPreferencesNotice] = useState<string | null>(null);
  const [isSavingPreferences, setIsSavingPreferences] = useState(false);

  const stageCopy = readStageCopy(
    onboardingStatus?.blockingStage ?? "loading",
    t,
  );
  const isReady = !onboardingLoading && onboardingStatus?.blockingStage === "ready";
  const canValidateProvider =
    !onboardingLoading &&
    onboardingStatus?.tokenPaired &&
    onboardingStatus?.configLoadable &&
    onboardingStatus?.providerConfigured;
  const needsTokenPairing =
    authRequired &&
    !onboardingLoading &&
    !onboardingStatus?.tokenPaired;
  const canConfigureProvider =
    !onboardingLoading &&
    onboardingStatus?.tokenPaired &&
    ["missing_config", "provider_setup", "provider_unreachable"].includes(
      onboardingStatus?.blockingStage ?? "",
    );

  useEffect(() => {
    setKind(onboardingStatus?.activeProvider ?? "");
    setModel(onboardingStatus?.activeModel ?? "");
    setBaseUrlOrEndpoint(
      onboardingStatus?.providerEndpoint ||
        onboardingStatus?.providerBaseUrl ||
        "",
    );
    setApiKey("");
    setSaveError(null);
    setValidationMessage(null);
    setValidationError(null);
    setPersonality(onboardingStatus?.personality || "calm_engineering");
    setMemoryProfile(onboardingStatus?.memoryProfile || "window_only");
    setPromptAddendum(onboardingStatus?.promptAddendum || "");
    setPreferencesError(null);
    setPreferencesNotice(null);
  }, [
    onboardingStatus?.activeModel,
    onboardingStatus?.activeProvider,
    onboardingStatus?.providerBaseUrl,
    onboardingStatus?.providerEndpoint,
    onboardingStatus?.personality,
    onboardingStatus?.memoryProfile,
    onboardingStatus?.promptAddendum,
    onboardingStatus?.blockingStage,
  ]);

  async function handleSaveProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaveError(null);
    setValidationMessage(null);
    setValidationError(null);
    setIsSaving(true);
    try {
      await onboardingApi.saveProvider({
        kind,
        model,
        baseUrlOrEndpoint,
        ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
      });
      clearOnboardingValidation();
      setApiKey("");
      refreshOnboardingStatus();
    } catch (error) {
      if (error instanceof ApiRequestError) {
        setSaveError(error.message);
      } else {
        setSaveError(t("onboarding.form.errors.saveFailed"));
      }
    } finally {
      setIsSaving(false);
    }
  }

  function handleSubmitToken(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const normalized = tokenInput.trim();
    if (!normalized) {
      return;
    }
    saveToken(normalized);
    setTokenInput("");
  }

  async function handleValidateProvider() {
    setValidationMessage(null);
    setValidationError(null);
    setIsValidating(true);
    try {
      const result = await onboardingApi.validateProvider();
      if (result.passed) {
        markOnboardingValidated();
        setValidationMessage(t("onboarding.validation.success"));
      } else {
        clearOnboardingValidation();
        setValidationError(
          t(`onboarding.validation.statuses.${result.credentialStatus}`, {
            defaultValue: t("onboarding.validation.failed"),
          }),
        );
      }
      refreshOnboardingStatus();
    } catch (error) {
      clearOnboardingValidation();
      if (error instanceof ApiRequestError) {
        setValidationError(error.message);
      } else {
        setValidationError(t("onboarding.validation.failed"));
      }
    } finally {
      setIsValidating(false);
    }
  }

  async function handleSavePreferences(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setPreferencesError(null);
    setPreferencesNotice(null);
    setIsSavingPreferences(true);
    try {
      await onboardingApi.savePreferences({
        personality,
        memoryProfile,
        ...(promptAddendum.trim()
          ? { promptAddendum: promptAddendum.trim() }
          : {}),
      });
      refreshOnboardingStatus();
      setPreferencesNotice(t("onboarding.preferences.saved"));
    } catch (error) {
      if (error instanceof ApiRequestError) {
        setPreferencesError(error.message);
      } else {
        setPreferencesError(t("onboarding.preferences.saveFailed"));
      }
    } finally {
      setIsSavingPreferences(false);
    }
  }

  return (
    <div className="page">
      <div className="hero-block">
        <div className="hero-eyebrow">{t("onboarding.eyebrow")}</div>
        <h1 className="hero-title">{stageCopy.title}</h1>
        <p className="hero-subtitle">{stageCopy.body}</p>
      </div>

      <Panel
        eyebrow={t("onboarding.panelEyebrow")}
        title={t("onboarding.panelTitle")}
      >
        <div className="dashboard-kv-grid">
          <div className="dashboard-kv-card">
            <span>{t("onboarding.summary.runtime")}</span>
            <strong>
              {onboardingStatus?.runtimeOnline
                ? t("onboarding.values.ready")
                : t("onboarding.values.blocked")}
            </strong>
          </div>
          <div className="dashboard-kv-card">
            <span>{t("onboarding.summary.token")}</span>
            <strong>
              {onboardingStatus?.tokenPaired
                ? t("onboarding.values.ready")
                : t("onboarding.values.blocked")}
            </strong>
          </div>
          <div className="dashboard-kv-card">
            <span>{t("onboarding.summary.config")}</span>
            <strong>
              {onboardingStatus?.configLoadable
                ? t("onboarding.values.ready")
                : t("onboarding.values.blocked")}
            </strong>
          </div>
          <div className="dashboard-kv-card">
            <span>{t("onboarding.summary.provider")}</span>
            <strong>
              {onboardingStatus?.providerReachable
                ? t("onboarding.values.ready")
                : t("onboarding.values.blocked")}
            </strong>
          </div>
        </div>

        <div className="dashboard-kv-list">
          <div className="dashboard-kv-row">
            <span>{t("onboarding.details.nextAction")}</span>
            <strong>
              {isReady
                ? t("onboarding.actions.enter_web")
                : t(`onboarding.actions.${onboardingStatus?.nextAction ?? "wait"}`)}
            </strong>
          </div>
          {onboardingStatus?.activeProvider ? (
            <div className="dashboard-kv-row">
              <span>{t("onboarding.details.provider")}</span>
              <strong>{onboardingStatus.activeProvider}</strong>
            </div>
          ) : null}
          {onboardingStatus?.activeModel ? (
            <div className="dashboard-kv-row">
              <span>{t("onboarding.details.model")}</span>
              <strong title={onboardingStatus.activeModel}>
                {onboardingStatus.activeModel}
              </strong>
            </div>
          ) : null}
          {onboardingStatus?.configPath ? (
            <div className="dashboard-kv-row">
              <span>{t("onboarding.details.configPath")}</span>
              <strong title={onboardingStatus.configPath}>
                {onboardingStatus.configPath}
              </strong>
            </div>
          ) : null}
        </div>

        {needsTokenPairing ? (
          <form className="settings-form onboarding-form" onSubmit={handleSubmitToken}>
            {autoPairingInProgress ? (
              <p className="settings-note onboarding-validation-note">
                {t("onboarding.tokenPairingAutoInProgress")}
              </p>
            ) : null}

            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-local-token">
                {status === "unauthorized"
                  ? t("auth.invalidTitle")
                  : t("auth.bannerTitle")}
              </label>
              <input
                id="onboarding-local-token"
                className="settings-input"
                type="password"
                autoComplete="off"
                value={tokenInput}
                onChange={(event) => setTokenInput(event.target.value)}
                placeholder={t("auth.inputPlaceholder")}
              />
              <p className="settings-helper">
                {status === "unauthorized"
                  ? t("auth.invalidBody", {
                      tokenPath: tokenPath ?? "",
                      tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN",
                    })
                  : t("auth.bannerBody", {
                      tokenPath: tokenPath ?? "",
                      tokenEnv: tokenEnv ?? "LOONGCLAW_WEB_TOKEN",
                    })}
              </p>
            </div>

            <div className="settings-actions onboarding-actions">
              <button type="submit" className="hero-btn hero-btn-primary">
                {t("auth.save")}
              </button>
              <button
                type="button"
                className="hero-btn hero-btn-secondary"
                onClick={clearToken}
              >
                {t("auth.clear")}
              </button>
            </div>
          </form>
        ) : null}

        {onboardingStatus?.tokenPaired ? (
          <div className="onboarding-optional-block">
            <button
              type="button"
              className="onboarding-optional-toggle"
              onClick={() => setShowOptionalSettings((current) => !current)}
            >
              <span>{t("onboarding.preferences.toggle")}</span>
              <strong>
                {showOptionalSettings
                  ? t("onboarding.preferences.hide")
                  : t("onboarding.preferences.show")}
              </strong>
            </button>

            {showOptionalSettings ? (
              <form
                className="settings-form onboarding-form onboarding-optional-form"
                onSubmit={handleSavePreferences}
              >
                <label className="settings-field">
                  <span className="settings-label">
                    {t("onboarding.preferences.personality")}
                  </span>
                  <select
                    className="settings-input"
                    value={personality}
                    onChange={(event) => setPersonality(event.target.value)}
                  >
                    <option value="calm_engineering">
                      {t("onboarding.preferences.personalityCalmEngineering")}
                    </option>
                    <option value="friendly_collab">
                      {t("onboarding.preferences.personalityFriendlyCollab")}
                    </option>
                    <option value="autonomous_executor">
                      {t("onboarding.preferences.personalityAutonomousExecutor")}
                    </option>
                  </select>
                </label>

                <label className="settings-field">
                  <span className="settings-label">
                    {t("onboarding.preferences.memoryProfile")}
                  </span>
                  <select
                    className="settings-input"
                    value={memoryProfile}
                    onChange={(event) => setMemoryProfile(event.target.value)}
                  >
                    <option value="window_only">
                      {t("onboarding.preferences.memoryProfileWindowOnly")}
                    </option>
                    <option value="window_plus_summary">
                      {t("onboarding.preferences.memoryProfileWindowPlusSummary")}
                    </option>
                    <option value="profile_plus_window">
                      {t("onboarding.preferences.memoryProfileProfilePlusWindow")}
                    </option>
                  </select>
                </label>

                <label className="settings-field">
                  <span className="settings-label">
                    {t("onboarding.preferences.promptAddendum")}
                  </span>
                  <textarea
                    className="settings-input settings-textarea"
                    value={promptAddendum}
                    onChange={(event) => setPromptAddendum(event.target.value)}
                    placeholder={t("onboarding.preferences.promptAddendumPlaceholder")}
                  />
                  <span className="settings-helper">
                    {t("onboarding.preferences.helper")}
                  </span>
                </label>

                {preferencesError ? (
                  <p className="settings-note dashboard-error">{preferencesError}</p>
                ) : null}
                {preferencesNotice ? (
                  <p className="settings-note">{preferencesNotice}</p>
                ) : null}

                <div className="settings-actions onboarding-actions">
                  <button
                    type="submit"
                    className="hero-btn hero-btn-secondary"
                    disabled={isSavingPreferences}
                  >
                    {isSavingPreferences
                      ? t("onboarding.preferences.savePending")
                      : t("onboarding.preferences.save")}
                  </button>
                </div>
              </form>
            ) : null}
          </div>
        ) : null}

        {canValidateProvider ? (
          <div className="settings-note onboarding-validation-note">
            {onboardingValidationSatisfied
              ? t("onboarding.validation.ready")
              : t("onboarding.validation.required")}
          </div>
        ) : null}

        {canConfigureProvider ? (
          <form className="settings-form onboarding-form" onSubmit={handleSaveProvider}>
            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-kind">
                {t("onboarding.form.kind")}
              </label>
              <input
                id="onboarding-provider-kind"
                className="settings-input"
                list="onboarding-provider-suggestions"
                value={kind}
                onChange={(event) => {
                  const nextKind = event.target.value;
                  const currentRoute =
                    onboardingStatus?.providerEndpoint ||
                    onboardingStatus?.providerBaseUrl ||
                    "";
                  setKind(nextKind);
                  if (baseUrlOrEndpoint === currentRoute) {
                    // Let the next provider fall back to its own default route unless
                    // the user has already typed a custom override.
                    setBaseUrlOrEndpoint("");
                  }
                }}
                placeholder={t("onboarding.form.kindPlaceholder")}
              />
              <datalist id="onboarding-provider-suggestions">
                <option value="openai" />
                <option value="volcengine" />
                <option value="deepseek" />
                <option value="anthropic" />
                <option value="openrouter" />
                <option value="ollama" />
                <option value="lmstudio" />
              </datalist>
            </div>

            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-model">
                {t("onboarding.form.model")}
              </label>
              <input
                id="onboarding-provider-model"
                className="settings-input"
                value={model}
                onChange={(event) => setModel(event.target.value)}
                placeholder={t("onboarding.form.modelPlaceholder")}
              />
            </div>

            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-route">
                {t("onboarding.form.baseUrlOrEndpoint")}
              </label>
              <input
                id="onboarding-provider-route"
                className="settings-input"
                value={baseUrlOrEndpoint}
                onChange={(event) => setBaseUrlOrEndpoint(event.target.value)}
                placeholder={t("onboarding.form.baseUrlOrEndpointPlaceholder")}
              />
              <p className="settings-helper">
                {t("onboarding.form.baseUrlOrEndpointHelper")}
              </p>
            </div>

            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-key">
                {t("onboarding.form.apiKey")}
              </label>
              <input
                id="onboarding-provider-key"
                className="settings-input"
                type="password"
                autoComplete="off"
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                placeholder={
                  onboardingStatus?.apiKeyConfigured
                    ? t("onboarding.form.apiKeyPlaceholderConfigured")
                    : t("onboarding.form.apiKeyPlaceholder")
                }
              />
              <p className="settings-helper">
                {onboardingStatus?.apiKeyConfigured
                  ? t("onboarding.form.apiKeyHelperConfigured")
                  : t("onboarding.form.apiKeyHelper")}
              </p>
            </div>

            {saveError ? <p className="settings-note dashboard-error">{saveError}</p> : null}

            <div className="settings-actions onboarding-actions">
              <button
                type="submit"
                className="hero-btn hero-btn-primary"
                disabled={isSaving}
              >
                {isSaving
                  ? t("onboarding.form.savePending")
                  : t("onboarding.form.save")}
              </button>
            </div>
          </form>
        ) : null}

        {validationError ? (
          <p className="settings-note dashboard-error">{validationError}</p>
        ) : null}

        {validationMessage ? (
          <p className="settings-note onboarding-validation-success">
            {validationMessage}
          </p>
        ) : null}

        {canValidateProvider && !onboardingValidationSatisfied ? (
          <div className="settings-actions onboarding-actions">
            <button
              type="button"
              className="hero-btn hero-btn-secondary"
              onClick={handleValidateProvider}
              disabled={isValidating}
            >
              {isValidating
                ? t("onboarding.validation.pending")
                : t("onboarding.validation.action")}
            </button>
          </div>
        ) : null}

        {isReady && onboardingValidationSatisfied ? (
          <div className="settings-actions onboarding-actions">
            <button
              type="button"
              className="hero-btn hero-btn-primary"
              onClick={acknowledgeOnboarding}
            >
              {t("onboarding.enter")}
            </button>
          </div>
        ) : null}
      </Panel>
    </div>
  );
}
