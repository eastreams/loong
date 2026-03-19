import { apiPost } from "../../../lib/api/client";
import type { ApiEnvelope } from "../../../lib/api/types";

interface SaveOnboardingProviderRequest {
  kind: string;
  model: string;
  baseUrlOrEndpoint: string;
  apiKey?: string;
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

export const onboardingApi = {
  async saveProvider(input: SaveOnboardingProviderRequest): Promise<void> {
    await apiPost<ApiEnvelope<Record<string, never>>, SaveOnboardingProviderRequest>(
      "/api/onboard/provider",
      input,
    );
  },
  async validateProvider(): Promise<OnboardingValidationResult> {
    const response = await apiPost<
      ApiEnvelope<OnboardingValidationResult>,
      Record<string, never>
    >("/api/onboard/validate", {});
    return response.data;
  },
};
