import { apiGet } from "../../../lib/api/client";
import type { ApiEnvelope } from "../../../lib/api/types";

export interface DashboardSummary {
  runtimeStatus: string;
  activeProvider: string | null;
  activeModel: string;
  memoryBackend: string;
  sessionCount: number;
  webInstallMode: string;
}

export interface DashboardProviderItem {
  id: string;
  label: string;
  enabled: boolean;
  model: string;
  endpoint: string;
  apiKeyConfigured: boolean;
  apiKeyMasked: string | null;
  defaultForKind: boolean;
}

interface DashboardProvidersResponse {
  activeProvider: string | null;
  items: DashboardProviderItem[];
}

export const dashboardApi = {
  async loadSummary(): Promise<DashboardSummary> {
    const response = await apiGet<ApiEnvelope<DashboardSummary>>(
      "/api/dashboard/summary",
    );
    return response.data;
  },

  async loadProviders(): Promise<DashboardProvidersResponse> {
    const response = await apiGet<ApiEnvelope<DashboardProvidersResponse>>(
      "/api/dashboard/providers",
    );
    return response.data;
  },
};
