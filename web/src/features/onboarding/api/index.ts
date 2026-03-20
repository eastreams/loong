import { apiPost } from "../../../lib/api/client";
import type { ApiEnvelope } from "../../../lib/api/types";

interface SaveOnboardingProviderRequest {
  kind: string;
  model: string;
  baseUrlOrEndpoint: string;
  apiKey?: string;
}

interface SaveOnboardingPreferencesRequest {
  personality: string;
  memoryProfile: string;
  promptAddendum?: string;
}

interface OnboardingStatusPayload {
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
  apiKeyConfigured: boolean;
  personality: string;
  memoryProfile: string;
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
  status: OnboardingStatusPayload;
}

interface OnboardingPairingResult {
  paired: boolean;
  mode: string;
  status: OnboardingStatusPayload;
}

export const onboardingApi = {
  async saveProvider(input: SaveOnboardingProviderRequest): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, SaveOnboardingProviderRequest>(
      "/api/onboard/provider",
      input,
    );
  },
  async applyProvider(
    input: SaveOnboardingProviderRequest,
  ): Promise<OnboardingValidationResult> {
    const response = await apiPost<
      ApiEnvelope<OnboardingValidationResult>,
      SaveOnboardingProviderRequest
    >("/api/onboard/provider/apply", input);
    return response.data;
  },
  async validateProvider(): Promise<OnboardingValidationResult> {
    const response = await apiPost<
      ApiEnvelope<OnboardingValidationResult>,
      Record<string, never>
    >("/api/onboard/validate", {});
    return response.data;
  },
  async autoPair(): Promise<OnboardingPairingResult> {
    const response = await apiPost<
      ApiEnvelope<OnboardingPairingResult>,
      Record<string, never>
    >("/api/onboard/pairing/auto", {});
    return response.data;
  },
  async clearPairing(): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, Record<string, never>>(
      "/api/onboard/pairing/clear",
      {},
    );
  },
  async savePreferences(input: SaveOnboardingPreferencesRequest): Promise<void> {
    await apiPost<
      ApiEnvelope<Record<string, never>>,
      SaveOnboardingPreferencesRequest
    >("/api/onboard/preferences", input);
  },
};
