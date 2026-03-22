import { useTranslation } from "react-i18next";
import { Panel } from "../../../components/surfaces/Panel";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { useOnboardingFlow } from "../hooks/useOnboardingFlow";
import {
  MEMORY_PROFILE_OPTIONS,
  PERSONALITY_OPTIONS,
  PROVIDER_KIND_SUGGESTIONS,
  usePreferencesForm,
  useProviderConfigForm,
} from "../providerConfig";

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
    case "session_refresh":
      return {
        title: t("onboarding.stages.sessionRefresh.title"),
        body: t("onboarding.stages.sessionRefresh.body"),
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
  const connection = useWebConnection();
  const {
    status,
    authRequired,
    tokenEnv,
    tokenPath,
    onboardingLoading,
    onboardingStatus,
    onboardingValidationSatisfied,
    acknowledgeOnboarding,
    autoPairingInProgress,
    authMode,
  } = connection;

  const providerForm = useProviderConfigForm({
    kind: onboardingStatus?.activeProvider ?? "",
    model: onboardingStatus?.activeModel ?? "",
    baseUrlOrEndpoint:
      onboardingStatus?.providerEndpoint || onboardingStatus?.providerBaseUrl || "",
    apiKeyConfigured: onboardingStatus?.apiKeyConfigured ?? false,
  });

  const preferencesForm = usePreferencesForm({
    personality: onboardingStatus?.personality || "calm_engineering",
    memoryProfile: onboardingStatus?.memoryProfile || "window_only",
    promptAddendum: onboardingStatus?.promptAddendum || "",
  });

  const { state, actions } = useOnboardingFlow({
    t,
    connection,
    providerForm,
    preferencesForm,
  });

  const {
    saveError,
    isSaving,
    validationMessage,
    validationError,
    isValidating,
    tokenInput,
    showOptionalSettings,
    preferencesError,
    preferencesNotice,
    isSavingPreferences,
  } = state;

  const {
    setTokenInput,
    setShowOptionalSettings,
    handleSaveProvider,
    handleSubmitToken,
    handleValidateProvider,
    handleSavePreferences,
    clearToken,
  } = actions;

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
    authMode !== "same_origin_session" &&
    !onboardingLoading &&
    !onboardingStatus?.tokenPaired;
  const canConfigureProvider =
    !onboardingLoading &&
    onboardingStatus?.tokenPaired &&
    ["missing_config", "provider_setup", "provider_unreachable"].includes(
      onboardingStatus?.blockingStage ?? "",
    );

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
            <span>
              {authMode === "same_origin_session"
                ? t("onboarding.summary.session")
                : t("onboarding.summary.token")}
            </span>
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
                    value={preferencesForm.personality}
                    onChange={(event) => preferencesForm.setPersonality(event.target.value)}
                  >
                    {PERSONALITY_OPTIONS.map((item) => (
                      <option key={item} value={item}>
                        {item === "calm_engineering"
                          ? t("onboarding.preferences.personalityCalmEngineering")
                          : item === "friendly_collab"
                            ? t("onboarding.preferences.personalityFriendlyCollab")
                            : t("onboarding.preferences.personalityAutonomousExecutor")}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="settings-field">
                  <span className="settings-label">
                    {t("onboarding.preferences.memoryProfile")}
                  </span>
                  <select
                    className="settings-input"
                    value={preferencesForm.memoryProfile}
                    onChange={(event) => preferencesForm.setMemoryProfile(event.target.value)}
                  >
                    {MEMORY_PROFILE_OPTIONS.map((item) => (
                      <option key={item} value={item}>
                        {item === "window_only"
                          ? t("onboarding.preferences.memoryProfileWindowOnly")
                          : item === "window_plus_summary"
                            ? t("onboarding.preferences.memoryProfileWindowPlusSummary")
                            : t("onboarding.preferences.memoryProfileProfilePlusWindow")}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="settings-field">
                  <span className="settings-label">
                    {t("onboarding.preferences.promptAddendum")}
                  </span>
                  <textarea
                    className="settings-input settings-textarea"
                    value={preferencesForm.promptAddendum}
                    onChange={(event) => preferencesForm.setPromptAddendum(event.target.value)}
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
                value={providerForm.kind}
                onChange={(event) => {
                  providerForm.setKindWithRouteReset(event.target.value);
                }}
                placeholder={t("onboarding.form.kindPlaceholder")}
              />
              <datalist id="onboarding-provider-suggestions">
                {PROVIDER_KIND_SUGGESTIONS.map((item) => (
                  <option key={item} value={item} />
                ))}
              </datalist>
            </div>

            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-model">
                {t("onboarding.form.model")}
              </label>
              <input
                id="onboarding-provider-model"
                className="settings-input"
                value={providerForm.model}
                onChange={(event) => providerForm.setModel(event.target.value)}
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
                value={providerForm.baseUrlOrEndpoint}
                onChange={(event) => providerForm.setBaseUrlOrEndpoint(event.target.value)}
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
                value={providerForm.apiKey}
                onChange={(event) => providerForm.setApiKeyValue(event.target.value)}
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
