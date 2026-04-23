import { useEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { ApiRequestError } from "../../../lib/api/client";
import type {
  SaveOnboardingPreferencesRequest,
  SaveOnboardingProviderRequest,
} from "../api";
import type { ProviderCatalogItem } from "./providerCatalog";

export const PERSONALITY_OPTIONS = [
  "calm_engineering",
  "friendly_collab",
  "autonomous_executor",
] as const;

const PERSONALITY_UI_ALIAS_MAP: Record<string, string> = {
  calm_engineering: "calm_engineering",
  classicist: "calm_engineering",
  friendly_collab: "friendly_collab",
  hermit: "friendly_collab",
  autonomous_executor: "autonomous_executor",
  pragmatist: "autonomous_executor",
};

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

interface FormResetOptions {
  force?: boolean;
}

function defaultRouteForKind(
  kind: string,
  catalog: ProviderCatalogItem[],
): string | null {
  const value = catalog.find((entry) => entry.kind === kind)?.defaultBaseUrl ?? "";
  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
}

function suggestedModelForKind(
  kind: string,
  catalog: ProviderCatalogItem[],
): string | null {
  const entry = catalog.find((item) => item.kind === kind);
  const value = entry?.recommendedOnboardingModel ?? entry?.defaultModel ?? "";
  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
}

function shouldAutoReplaceRouteOnKindSwitch(params: {
  currentRoute: string;
  currentKind: string;
  sourceRoute: string;
  nextKind: string;
  catalog: ProviderCatalogItem[];
}): boolean {
  const {
    currentRoute,
    currentKind,
    sourceRoute,
    nextKind,
    catalog,
  } = params;
  const normalizedCurrentRoute = currentRoute.trim();
  if (!normalizedCurrentRoute) {
    return true;
  }

  const normalizedSourceRoute = sourceRoute.trim();
  if (normalizedCurrentRoute === normalizedSourceRoute) {
    return true;
  }

  const currentDefaultRoute = defaultRouteForKind(currentKind, catalog);
  if (currentDefaultRoute && normalizedCurrentRoute === currentDefaultRoute) {
    return true;
  }

  const nextDefaultRoute = defaultRouteForKind(nextKind, catalog);
  if (nextDefaultRoute && normalizedCurrentRoute === nextDefaultRoute) {
    return true;
  }

  return false;
}

function shouldAutoReplaceModelOnKindSwitch(params: {
  currentModel: string;
  currentKind: string;
  sourceModel: string;
  nextKind: string;
  catalog: ProviderCatalogItem[];
}): boolean {
  const {
    currentModel,
    currentKind,
    sourceModel,
    nextKind,
    catalog,
  } = params;
  const normalizedCurrentModel = currentModel.trim();
  if (!normalizedCurrentModel) {
    return true;
  }

  const normalizedSourceModel = sourceModel.trim();
  if (normalizedCurrentModel === normalizedSourceModel) {
    return true;
  }

  const currentSuggestedModel = suggestedModelForKind(currentKind, catalog);
  if (currentSuggestedModel && normalizedCurrentModel === currentSuggestedModel) {
    return true;
  }

  const nextSuggestedModel = suggestedModelForKind(nextKind, catalog);
  if (nextSuggestedModel && normalizedCurrentModel === nextSuggestedModel) {
    return true;
  }

  return false;
}


export function useProviderConfigForm(
  source: ProviderConfigFormSource,
  providerCatalog: ProviderCatalogItem[] = [],
) {
  const sourceRef = useRef(source);
  const catalogRef = useRef(providerCatalog);
  const [kind, setKind] = useState(source.kind);
  const [model, setModel] = useState(source.model);
  const [baseUrlOrEndpoint, setBaseUrlOrEndpoint] = useState(source.baseUrlOrEndpoint);
  const [apiKey, setApiKey] = useState("");
  const [apiKeyDirty, setApiKeyDirty] = useState(false);
  const [kindDirty, setKindDirty] = useState(false);
  const [modelDirty, setModelDirty] = useState(false);
  const [baseUrlDirty, setBaseUrlDirty] = useState(false);

  const apiKeyDirtyRef = useRef(false);
  const kindDirtyRef = useRef(false);
  const modelDirtyRef = useRef(false);
  const baseUrlDirtyRef = useRef(false);

  useEffect(() => {
    catalogRef.current = providerCatalog;
  }, [providerCatalog]);

  function updateApiKeyDirty(nextDirty: boolean) {
    apiKeyDirtyRef.current = nextDirty;
    setApiKeyDirty(nextDirty);
  }

  function updateKindDirty(nextDirty: boolean) {
    kindDirtyRef.current = nextDirty;
    setKindDirty(nextDirty);
  }

  function updateModelDirty(nextDirty: boolean) {
    modelDirtyRef.current = nextDirty;
    setModelDirty(nextDirty);
  }

  function updateBaseUrlDirty(nextDirty: boolean) {
    baseUrlDirtyRef.current = nextDirty;
    setBaseUrlDirty(nextDirty);
  }

  function resetFromSource(
    nextSource: ProviderConfigFormSource,
    options?: FormResetOptions,
  ) {
    const force = options?.force ?? false;
    sourceRef.current = nextSource;

    if (force || !kindDirtyRef.current) {
      setKind(nextSource.kind);
      if (force) {
        updateKindDirty(false);
      }
    }

    if (force || !modelDirtyRef.current) {
      setModel(nextSource.model);
      if (force) {
        updateModelDirty(false);
      }
    }

    if (force || !baseUrlDirtyRef.current) {
      setBaseUrlOrEndpoint(nextSource.baseUrlOrEndpoint);
      if (force) {
        updateBaseUrlDirty(false);
      }
    }

    if (force || !apiKeyDirtyRef.current) {
      setApiKey("");
      updateApiKeyDirty(false);
    }
  }

  useEffect(() => {
    resetFromSource(source);
  }, [source.baseUrlOrEndpoint, source.kind, source.model]);

function setKindWithRouteReset(nextKind: string) {
  const currentKind = kind;
  const currentCatalog = catalogRef.current;
  setKind(nextKind);
  updateKindDirty(nextKind !== sourceRef.current.kind);
  const defaultRoute = defaultRouteForKind(nextKind, currentCatalog);
  setBaseUrlOrEndpoint((current) => {
    const shouldReplaceRoute = shouldAutoReplaceRouteOnKindSwitch({
      currentRoute: current,
      currentKind,
      sourceRoute: sourceRef.current.baseUrlOrEndpoint,
      nextKind,
      catalog: currentCatalog,
    });
    const nextValue =
      defaultRoute && shouldReplaceRoute ? defaultRoute : current;
    updateBaseUrlDirty(nextValue !== sourceRef.current.baseUrlOrEndpoint);
    return nextValue;
  });
  const suggestedModel = suggestedModelForKind(nextKind, currentCatalog);
  setModel((current) => {
    const shouldReplaceModel =
      !modelDirtyRef.current ||
      shouldAutoReplaceModelOnKindSwitch({
        currentModel: current,
        currentKind,
        sourceModel: sourceRef.current.model,
        nextKind,
        catalog: currentCatalog,
      });
    const nextValue =
      suggestedModel && shouldReplaceModel ? suggestedModel : current;
    updateModelDirty(nextValue !== sourceRef.current.model);
    return nextValue;
  });
}


  function setModelValue(nextModel: string) {
    setModel(nextModel);
    updateModelDirty(nextModel !== sourceRef.current.model);
  }

  function setBaseUrlValue(nextBaseUrlOrEndpoint: string) {
    setBaseUrlOrEndpoint(nextBaseUrlOrEndpoint);
    updateBaseUrlDirty(nextBaseUrlOrEndpoint !== sourceRef.current.baseUrlOrEndpoint);
  }

  function setApiKeyValue(nextApiKey: string) {
    setApiKey(nextApiKey);
    updateApiKeyDirty(true);
  }

  function handleApiKeyFocus() {
    if (sourceRef.current.apiKeyConfigured && !apiKeyDirtyRef.current) {
      setApiKey("");
      updateApiKeyDirty(true);
    }
  }

  function markApiKeyPristine() {
    setApiKey("");
    updateApiKeyDirty(false);
  }

  return {
    kind,
    model,
    baseUrlOrEndpoint,
    apiKey,
    apiKeyDirty,
    isDirty: kindDirty || modelDirty || baseUrlDirty || apiKeyDirty,
    resetFromSource,
    setModel: setModelValue,
    setBaseUrlOrEndpoint: setBaseUrlValue,
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
  slidingWindow: number;
  promptAddendum: string;
}

export function normalizePersonalityForUi(personality: string): string {
  const normalized = personality.trim();
  return PERSONALITY_UI_ALIAS_MAP[normalized] ?? normalized;
}

export function usePreferencesForm(source: PreferencesFormSource) {
  const sourceRef = useRef(source);
  const [personality, setPersonality] = useState(
    normalizePersonalityForUi(source.personality),
  );
  const [memoryProfile, setMemoryProfile] = useState(source.memoryProfile);
  const [slidingWindow, setSlidingWindow] = useState(String(source.slidingWindow));
  const [promptAddendum, setPromptAddendum] = useState(source.promptAddendum);
  const [personalityDirty, setPersonalityDirty] = useState(false);
  const [memoryProfileDirty, setMemoryProfileDirty] = useState(false);
  const [slidingWindowDirty, setSlidingWindowDirty] = useState(false);
  const [promptAddendumDirty, setPromptAddendumDirty] = useState(false);

  const personalityDirtyRef = useRef(false);
  const memoryProfileDirtyRef = useRef(false);
  const slidingWindowDirtyRef = useRef(false);
  const promptAddendumDirtyRef = useRef(false);

  function updatePersonalityDirty(nextDirty: boolean) {
    personalityDirtyRef.current = nextDirty;
    setPersonalityDirty(nextDirty);
  }

  function updateMemoryProfileDirty(nextDirty: boolean) {
    memoryProfileDirtyRef.current = nextDirty;
    setMemoryProfileDirty(nextDirty);
  }

  function updateSlidingWindowDirty(nextDirty: boolean) {
    slidingWindowDirtyRef.current = nextDirty;
    setSlidingWindowDirty(nextDirty);
  }

  function updatePromptAddendumDirty(nextDirty: boolean) {
    promptAddendumDirtyRef.current = nextDirty;
    setPromptAddendumDirty(nextDirty);
  }

  function resetFromSource(
    nextSource: PreferencesFormSource,
    options?: FormResetOptions,
  ) {
    const force = options?.force ?? false;
    sourceRef.current = nextSource;
    const nextPersonality = normalizePersonalityForUi(nextSource.personality);

    if (force || !personalityDirtyRef.current) {
      setPersonality(nextPersonality);
      if (force) {
        updatePersonalityDirty(false);
      }
    }

    if (force || !memoryProfileDirtyRef.current) {
      setMemoryProfile(nextSource.memoryProfile);
      if (force) {
        updateMemoryProfileDirty(false);
      }
    }

    if (force || !slidingWindowDirtyRef.current) {
      setSlidingWindow(String(nextSource.slidingWindow));
      if (force) {
        updateSlidingWindowDirty(false);
      }
    }

    if (force || !promptAddendumDirtyRef.current) {
      setPromptAddendum(nextSource.promptAddendum);
      if (force) {
        updatePromptAddendumDirty(false);
      }
    }
  }

  useEffect(() => {
    resetFromSource(source);
  }, [
    source.memoryProfile,
    source.personality,
    source.promptAddendum,
    source.slidingWindow,
  ]);

  function setPersonalityValue(nextPersonality: string) {
    setPersonality(nextPersonality);
    updatePersonalityDirty(
      nextPersonality !== normalizePersonalityForUi(sourceRef.current.personality),
    );
  }

  function setMemoryProfileValue(nextMemoryProfile: string) {
    setMemoryProfile(nextMemoryProfile);
    updateMemoryProfileDirty(nextMemoryProfile !== sourceRef.current.memoryProfile);
  }

  function setSlidingWindowValue(nextSlidingWindow: string) {
    setSlidingWindow(nextSlidingWindow);
    updateSlidingWindowDirty(nextSlidingWindow !== String(sourceRef.current.slidingWindow));
  }

  function setPromptAddendumValue(nextPromptAddendum: string) {
    setPromptAddendum(nextPromptAddendum);
    updatePromptAddendumDirty(
      nextPromptAddendum !== sourceRef.current.promptAddendum,
    );
  }

  return {
    personality,
    memoryProfile,
    slidingWindow,
    promptAddendum,
    isDirty:
      personalityDirty ||
      memoryProfileDirty ||
      slidingWindowDirty ||
      promptAddendumDirty,
    resetFromSource,
    setPersonality: setPersonalityValue,
    setMemoryProfile: setMemoryProfileValue,
    setSlidingWindow: setSlidingWindowValue,
    setPromptAddendum: setPromptAddendumValue,
  };
}

export function buildPreferencesSavePayload(input: {
  personality: string;
  memoryProfile: string;
  slidingWindow: string;
  promptAddendum: string;
}, t: TFunction): SaveOnboardingPreferencesRequest {
  const normalizedSlidingWindow = input.slidingWindow.trim();
  if (!/^\d+$/.test(normalizedSlidingWindow)) {
    throw new Error(t("onboarding.preferences.errors.slidingWindowInvalid"));
  }

  const slidingWindow = Number(normalizedSlidingWindow);
  if (!Number.isInteger(slidingWindow) || slidingWindow < 1 || slidingWindow > 128) {
    throw new Error(t("onboarding.preferences.errors.slidingWindowInvalid"));
  }

  const payload: SaveOnboardingPreferencesRequest = {
    personality: input.personality,
    memoryProfile: input.memoryProfile,
    slidingWindow,
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
  if (error instanceof Error) {
    return error.message;
  }
  return t(fallbackKey);
}
