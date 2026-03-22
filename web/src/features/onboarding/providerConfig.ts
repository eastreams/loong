import { useEffect, useState } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../lib/api/client";
import type {
  SaveOnboardingPreferencesRequest,
  SaveOnboardingProviderRequest,
} from "./api";


export const PROVIDER_KIND_SUGGESTIONS = [
  "openai",
  "volcengine",
  "deepseek",
  "anthropic",
  "openrouter",
  "ollama",
  "lmstudio",
] as const;

export const PERSONALITY_OPTIONS = [
  "calm_engineering",
  "friendly_collab",
  "autonomous_executor",
] as const;

export const MEMORY_PROFILE_OPTIONS = [
  "window_only",
  "window_plus_summary",
  "profile_plus_window",
] as const;


export interface ProviderConfigFormSource {
  kind: string;
  model: string;
  baseUrlOrEndpoint: string;
  apiKeyConfigured: boolean;
}

export function useProviderConfigForm(source: ProviderConfigFormSource) {
  const [kind, setKind] = useState(source.kind);
  const [model, setModel] = useState(source.model);
  const [baseUrlOrEndpoint, setBaseUrlOrEndpoint] = useState(source.baseUrlOrEndpoint);
  const [apiKey, setApiKey] = useState("");
  const [apiKeyDirty, setApiKeyDirty] = useState(false);

  function resetFromSource(nextSource: ProviderConfigFormSource) {
    setKind(nextSource.kind);
    setModel(nextSource.model);
    setBaseUrlOrEndpoint(nextSource.baseUrlOrEndpoint);
    setApiKey("");
    setApiKeyDirty(false);
  }

  useEffect(() => {
    resetFromSource(source);
  }, [source.baseUrlOrEndpoint, source.kind, source.model]);

  function setKindWithRouteReset(nextKind: string) {
    setKind(nextKind);
    setBaseUrlOrEndpoint((current) =>
      current === source.baseUrlOrEndpoint ? "" : current,
    );
  }

  function setApiKeyValue(nextApiKey: string) {
    setApiKey(nextApiKey);
    setApiKeyDirty(true);
  }

  function handleApiKeyFocus() {
    if (source.apiKeyConfigured && !apiKeyDirty) {
      setApiKey("");
      setApiKeyDirty(true);
    }
  }

  function markApiKeyPristine() {
    setApiKey("");
    setApiKeyDirty(false);
  }

  return {
    kind,
    model,
    baseUrlOrEndpoint,
    apiKey,
    apiKeyDirty,
    resetFromSource,
    setModel,
    setBaseUrlOrEndpoint,
    setKindWithRouteReset,
    setApiKeyValue,
    handleApiKeyFocus,
    markApiKeyPristine,
  };
}

export function buildProviderSavePayload(input: {
  kind: string;
  model: string;
  baseUrlOrEndpoint: string;
  apiKey: string;
}): SaveOnboardingProviderRequest {
  const payload: SaveOnboardingProviderRequest = {
    kind: input.kind.trim(),
    model: input.model.trim(),
    baseUrlOrEndpoint: input.baseUrlOrEndpoint.trim(),
  };

  const normalizedApiKey = input.apiKey.trim();
  if (normalizedApiKey) {
    payload.apiKey = normalizedApiKey;
  }

  return payload;
}

export interface PreferencesFormSource {
  personality: string;
  memoryProfile: string;
  promptAddendum: string;
}

export function usePreferencesForm(source: PreferencesFormSource) {
  const [personality, setPersonality] = useState(source.personality);
  const [memoryProfile, setMemoryProfile] = useState(source.memoryProfile);
  const [promptAddendum, setPromptAddendum] = useState(source.promptAddendum);

  function resetFromSource(nextSource: PreferencesFormSource) {
    setPersonality(nextSource.personality);
    setMemoryProfile(nextSource.memoryProfile);
    setPromptAddendum(nextSource.promptAddendum);
  }

  useEffect(() => {
    resetFromSource(source);
  }, [source.memoryProfile, source.personality, source.promptAddendum]);

  return {
    personality,
    memoryProfile,
    promptAddendum,
    resetFromSource,
    setPersonality,
    setMemoryProfile,
    setPromptAddendum,
  };
}

export function buildPreferencesSavePayload(input: {
  personality: string;
  memoryProfile: string;
  promptAddendum: string;
}): SaveOnboardingPreferencesRequest {
  const payload: SaveOnboardingPreferencesRequest = {
    personality: input.personality,
    memoryProfile: input.memoryProfile,
  };

  const normalizedPromptAddendum = input.promptAddendum.trim();
  if (normalizedPromptAddendum) {
    payload.promptAddendum = normalizedPromptAddendum;
  }

  return payload;
}

export function readProviderValidationFailure(

  credentialStatus: string,
  t: TFunction,
): string {
  return t(`onboarding.validation.statuses.${credentialStatus}`, {
    defaultValue: t("onboarding.validation.failed"),
  });
}

export function readProviderSaveError(
  error: unknown,
  t: TFunction,
  fallbackKey: string,
): string {
  if (error instanceof ApiRequestError) {
    return error.message;
  }
  return t(fallbackKey);
}
