import {
  apiGetData,
  apiPost,
  apiPostData,
  type ApiRequestOptions,
} from "../../../lib/api/client";
import type { ApiEnvelope } from "../../../lib/api/types";
import type { ProviderCatalogItem } from "../provider/providerCatalog";

const ONBOARDING_READ_TIMEOUT_MS = 10_000;
const ONBOARDING_WRITE_TIMEOUT_MS = 30_000;

export interface MetaAuthInfo {
  required: boolean;
  scheme: string;
  header: string;
  tokenPath: string;
  tokenEnv: string;
  mode: string;
}

export interface WebMetaPayload {
  appVersion: string;
  apiVersion: string;
  webInstallMode: string;
  supportedLocales: string[];
  defaultLocale: string;
  auth: MetaAuthInfo;
}

export interface SaveOnboardingProviderRequest {
  kind: string;
  model: string;
  baseUrlOrEndpoint: string;
  apiKey?: string;
}

export interface SaveOnboardingPreferencesRequest {
  personality: string;
  memoryProfile: string;
  slidingWindow?: number;
  promptAddendum?: string;
}

export interface OnboardingStatus {
  runtimeOnline: boolean;
  tokenRequired: boolean;
  tokenPaired: boolean;
  configExists: boolean;
  configLoadable: boolean;
  providerConfigured: boolean;
  providerReachable: boolean;
  activeProvider: string | null;
  activeModel: string;
  providerBaseUrl: string;
  providerEndpoint: string;
  providerEndpointExplicit: boolean;
  apiKeyConfigured: boolean;
  personality: string;
  memoryProfile: string;
  slidingWindow: number;
  promptAddendum: string;
  configPath: string;
  blockingStage: string;
  nextAction: string;
}

export interface OnboardingValidationResult {
  passed: boolean;
  endpointStatus: string;
  endpointStatusCode: number | null;
  credentialStatus: string;
  credentialStatusCode: number | null;
  status: OnboardingStatus;
}

interface ProviderCatalogResponse {
  items: ProviderCatalogItem[];
}

interface OnboardingPairingResult {
  paired: boolean;
  mode: string;
  status: OnboardingStatus;
}

function withDefaultTimeout(
  request: ApiRequestOptions | undefined,
  timeoutMs: number,
): ApiRequestOptions {
  return {
    ...request,
    timeoutMs: request?.timeoutMs ?? timeoutMs,
  };
}

export const onboardingApi = {
  async loadMeta(request?: ApiRequestOptions): Promise<WebMetaPayload> {
    return apiGetData<WebMetaPayload>(
      "/api/meta",
      withDefaultTimeout(request, ONBOARDING_READ_TIMEOUT_MS),
    );
  },

  async loadStatus(request?: ApiRequestOptions): Promise<OnboardingStatus> {
    return apiGetData<OnboardingStatus>(
      "/api/onboard/status",
      withDefaultTimeout(request, ONBOARDING_READ_TIMEOUT_MS),
    );
  },

  async loadProviderCatalog(request?: ApiRequestOptions): Promise<ProviderCatalogItem[]> {
    const response = await apiGetData<ProviderCatalogResponse>(
      "/api/providers/catalog",
      withDefaultTimeout(request, ONBOARDING_READ_TIMEOUT_MS),
    );
    return response.items;
  },

  async saveProvider(
    input: SaveOnboardingProviderRequest,
    request?: ApiRequestOptions,
  ): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, SaveOnboardingProviderRequest>(
      "/api/onboard/provider",
      input,
      withDefaultTimeout(request, ONBOARDING_WRITE_TIMEOUT_MS),
    );
  },

  async applyProvider(
    input: SaveOnboardingProviderRequest,
    request?: ApiRequestOptions,
  ): Promise<OnboardingValidationResult> {
    return apiPostData<OnboardingValidationResult, SaveOnboardingProviderRequest>(
      "/api/onboard/provider/apply",
      input,
      withDefaultTimeout(request, ONBOARDING_WRITE_TIMEOUT_MS),
    );
  },

  async validateProvider(request?: ApiRequestOptions): Promise<OnboardingValidationResult> {
    return apiPostData<OnboardingValidationResult, Record<string, never>>(
      "/api/onboard/validate",
      {},
      withDefaultTimeout(request, ONBOARDING_WRITE_TIMEOUT_MS),
    );
  },

  async autoPair(request?: ApiRequestOptions): Promise<OnboardingPairingResult> {
    return apiPostData<OnboardingPairingResult, Record<string, never>>(
      "/api/onboard/pairing/auto",
      {},
      withDefaultTimeout(request, ONBOARDING_READ_TIMEOUT_MS),
    );
  },

  async clearPairing(request?: ApiRequestOptions): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, Record<string, never>>(
      "/api/onboard/pairing/clear",
      {},
      withDefaultTimeout(request, ONBOARDING_WRITE_TIMEOUT_MS),
    );
  },

  async savePreferences(
    input: SaveOnboardingPreferencesRequest,
    request?: ApiRequestOptions,
  ): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, SaveOnboardingPreferencesRequest>(
      "/api/onboard/preferences",
      input,
      withDefaultTimeout(request, ONBOARDING_WRITE_TIMEOUT_MS),
    );
  },
};
