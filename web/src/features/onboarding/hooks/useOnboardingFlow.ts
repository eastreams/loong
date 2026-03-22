import { useState, useEffect, type FormEvent } from "react";
import type { TFunction } from "i18next";
import { onboardingApi } from "../api";
import {
  buildPreferencesSavePayload,
  buildProviderSavePayload,
  readProviderSaveError,
  readProviderValidationFailure,
  usePreferencesForm,
  useProviderConfigForm,
} from "../providerConfig";
import type { WebSessionContextValue } from "../../../contexts/WebSessionContext";

interface UseOnboardingFlowParams {
  t: TFunction;
  connection: WebSessionContextValue;
  providerForm: ReturnType<typeof useProviderConfigForm>;
  preferencesForm: ReturnType<typeof usePreferencesForm>;
}

export function useOnboardingFlow({
  t,
  connection,
  providerForm,
  preferencesForm,
}: UseOnboardingFlowParams) {
  const {
    saveToken,
    clearToken,
    acceptValidatedOnboardingStatus,
    clearOnboardingValidation,
    refreshOnboardingStatus,
    markOnboardingValidated,
    onboardingStatus,
  } = connection;

  const [saveError, setSaveError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [validationMessage, setValidationMessage] = useState<string | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [tokenInput, setTokenInput] = useState("");
  const [showOptionalSettings, setShowOptionalSettings] = useState(false);
  const [preferencesError, setPreferencesError] = useState<string | null>(null);
  const [preferencesNotice, setPreferencesNotice] = useState<string | null>(null);
  const [isSavingPreferences, setIsSavingPreferences] = useState(false);

  useEffect(() => {
    setSaveError(null);
    setValidationMessage(null);
    setValidationError(null);
    setPreferencesError(null);
    setPreferencesNotice(null);
  }, [onboardingStatus?.blockingStage]);

  async function handleSaveProvider(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaveError(null);
    setValidationMessage(null);
    setValidationError(null);
    setIsSaving(true);
    try {
      const result = await onboardingApi.applyProvider(
        buildProviderSavePayload({
          kind: providerForm.kind,
          model: providerForm.model,
          baseUrlOrEndpoint: providerForm.baseUrlOrEndpoint,
          apiKey: providerForm.apiKey,
        }),
      );

      providerForm.markApiKeyPristine();
      if (result.passed) {
        acceptValidatedOnboardingStatus(result.status);
        setValidationMessage(t("onboarding.validation.success"));
      } else {
        clearOnboardingValidation();
        setValidationError(readProviderValidationFailure(result.credentialStatus, t));
        refreshOnboardingStatus();
      }
    } catch (error) {
      setSaveError(readProviderSaveError(error, t, "onboarding.form.errors.saveFailed"));
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
        setValidationError(readProviderValidationFailure(result.credentialStatus, t));
      }
      refreshOnboardingStatus();
    } catch (error) {
      clearOnboardingValidation();
      setValidationError(readProviderSaveError(error, t, "onboarding.validation.failed"));
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
      await onboardingApi.savePreferences(
        buildPreferencesSavePayload({
          personality: preferencesForm.personality,
          memoryProfile: preferencesForm.memoryProfile,
          promptAddendum: preferencesForm.promptAddendum,
        }),
      );
      refreshOnboardingStatus();
      setPreferencesNotice(t("onboarding.preferences.saved"));
    } catch (error) {
      setPreferencesError(readProviderSaveError(error, t, "onboarding.preferences.saveFailed"));
    } finally {
      setIsSavingPreferences(false);
    }
  }

  return {
    state: {
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
    },
    actions: {
      setTokenInput,
      setShowOptionalSettings,
      handleSaveProvider,
      handleSubmitToken,
      handleValidateProvider,
      handleSavePreferences,
      clearToken,
    },
  };
}
