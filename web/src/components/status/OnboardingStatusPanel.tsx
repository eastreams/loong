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
    onboardingStatus?.blockingStage === "token_pairing";
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
  }, [
    onboardingStatus?.activeModel,
    onboardingStatus?.activeProvider,
    onboardingStatus?.providerBaseUrl,
    onboardingStatus?.providerEndpoint,
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
            <div className="settings-field">
              <label className="settings-label" htmlFor="onboarding-provider-kind">
                {t("onboarding.form.kind")}
              </label>
              <input
                id="onboarding-provider-kind"
                className="settings-input"
                list="onboarding-provider-suggestions"
                value={kind}
                onChange={(event) => setKind(event.target.value)}
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
