import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
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

interface ChoiceFieldOption {
  value: string;
  label: string;
}

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

function ChoiceField(props: {
  id: string;
  label: string;
  value: string;
  placeholder?: string;
  options: ChoiceFieldOption[];
  onSelect: (value: string) => void;
}) {
  const { id, label, value, placeholder, options, onSelect } = props;
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const activeOption =
    options.find((option) => option.value === value) ??
    (value ? { value, label: value } : null);

  useEffect(() => {
    if (!open) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOpen(false);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [open]);

  return (
    <div className="settings-field">
      <label className="settings-label" htmlFor={id}>
        {label}
      </label>
      <div className="settings-choice-shell" ref={menuRef}>
        <button
          id={id}
          type="button"
          className={`settings-input settings-choice-button${open ? " is-open" : ""}`}
          aria-haspopup="listbox"
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
        >
          <span>{activeOption?.label ?? placeholder ?? ""}</span>
          <ChevronDown size={16} className="settings-choice-icon" />
        </button>
        {open ? (
          <div className="settings-choice-menu" role="listbox">
            {options.map((option) => (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={value === option.value}
                className={`settings-choice-option${
                  value === option.value ? " is-selected" : ""
                }`}
                onClick={() => {
                  onSelect(option.value);
                  setOpen(false);
                }}
              >
                {option.label}
              </button>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
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
  const needsTokenPairing =
    authRequired &&
    authMode !== "same_origin_session" &&
    !onboardingLoading &&
    !onboardingStatus?.tokenPaired;
  const canConfigureProvider =
    !onboardingLoading &&
    onboardingStatus?.tokenPaired &&
    [
      "missing_config",
      "config_invalid",
      "provider_setup",
      "provider_unreachable",
    ].includes(
      onboardingStatus?.blockingStage ?? "",
    );
  const providerKindOptions = PROVIDER_KIND_SUGGESTIONS.includes(
    providerForm.kind as (typeof PROVIDER_KIND_SUGGESTIONS)[number],
  )
    ? PROVIDER_KIND_SUGGESTIONS.map((item) => ({
        value: item,
        label: item,
      }))
    : providerForm.kind
      ? [
          { value: providerForm.kind, label: providerForm.kind },
          ...PROVIDER_KIND_SUGGESTIONS.map((item) => ({
            value: item,
            label: item,
          })),
        ]
      : PROVIDER_KIND_SUGGESTIONS.map((item) => ({
          value: item,
          label: item,
        }));
  const personalityOptions = PERSONALITY_OPTIONS.map((item) => ({
    value: item,
    label:
      item === "calm_engineering"
        ? t("onboarding.preferences.personalityCalmEngineering")
        : item === "friendly_collab"
          ? t("onboarding.preferences.personalityFriendlyCollab")
          : t("onboarding.preferences.personalityAutonomousExecutor"),
  }));
  const memoryProfileOptions = MEMORY_PROFILE_OPTIONS.map((item) => ({
    value: item,
    label:
      item === "window_only"
        ? t("onboarding.preferences.memoryProfileWindowOnly")
        : item === "window_plus_summary"
          ? t("onboarding.preferences.memoryProfileWindowPlusSummary")
          : t("onboarding.preferences.memoryProfileProfilePlusWindow"),
  }));
  const canValidateProvider =
    !canConfigureProvider &&
    !onboardingLoading &&
    onboardingStatus?.tokenPaired &&
    onboardingStatus?.configLoadable &&
    onboardingStatus?.providerConfigured;

  return (
    <div className="page">
      <div className="hero-block">
        <div className="hero-eyebrow">{t("onboarding.eyebrow")}</div>
        <h1 className="hero-title">{stageCopy.title}</h1>
      </div>

      <Panel title={t("onboarding.panelTitle")}>
        <div className="dashboard-kv-grid onboarding-summary-grid">
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

        {onboardingStatus?.configPath ? (
          <div className="dashboard-kv-list">
            <div className="dashboard-kv-row">
              <span>{t("onboarding.details.configPath")}</span>
              <strong title={onboardingStatus.configPath}>
                {onboardingStatus.configPath}
              </strong>
            </div>
          </div>
        ) : null}

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


        {canValidateProvider ? (
          <div className="settings-note onboarding-validation-note">
            {onboardingValidationSatisfied
              ? t("onboarding.validation.ready")
              : t("onboarding.validation.required")}
          </div>
        ) : null}

        {canConfigureProvider ? (
          <form className="settings-form onboarding-form" onSubmit={handleSaveProvider}>
            <ChoiceField
              id="onboarding-provider-kind"
              label={t("onboarding.form.kind")}
              value={providerForm.kind}
              placeholder={t("onboarding.form.kindPlaceholder")}
              options={providerKindOptions}
              onSelect={providerForm.setKindWithRouteReset}
            />

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
            </div>

            {onboardingStatus?.tokenPaired ? (
              <div className="settings-field">
                <span className="settings-label">{t("onboarding.preferences.toggle")}</span>
                <div
                  className={`onboarding-optional-block${showOptionalSettings ? " is-open" : ""}`}
                >
                  <button
                    type="button"
                    className="onboarding-optional-toggle"
                    onClick={() => setShowOptionalSettings((current) => !current)}
                    aria-expanded={showOptionalSettings}
                  >
                    <strong>
                      {showOptionalSettings
                        ? t("onboarding.preferences.hide")
                        : t("onboarding.preferences.show")}
                    </strong>
                    <ChevronDown
                      size={16}
                      className={`onboarding-optional-toggle-icon${
                        showOptionalSettings ? " is-open" : ""
                      }`}
                    />
                  </button>

                  {showOptionalSettings ? (
                    <div className="settings-form onboarding-form onboarding-optional-form">
                    <ChoiceField
                      id="onboarding-preferences-personality"
                      label={t("onboarding.preferences.personality")}
                      value={preferencesForm.personality}
                      options={personalityOptions}
                      onSelect={preferencesForm.setPersonality}
                    />

                    <ChoiceField
                      id="onboarding-preferences-memory-profile"
                      label={t("onboarding.preferences.memoryProfile")}
                      value={preferencesForm.memoryProfile}
                      options={memoryProfileOptions}
                      onSelect={preferencesForm.setMemoryProfile}
                    />

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
                          type="button"
                          className="hero-btn hero-btn-secondary"
                          disabled={isSavingPreferences}
                          onClick={() => {
                            void handleSavePreferences();
                          }}
                        >
                          {isSavingPreferences
                            ? t("onboarding.preferences.savePending")
                            : t("onboarding.preferences.save")}
                        </button>
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>
            ) : null}
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

