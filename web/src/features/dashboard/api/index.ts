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

export interface DashboardRuntime {
  status: string;
  source: string;
  configPath: string;
  memoryBackend: string;
  memoryMode: string;
  ingestMode: string;
  webInstallMode: string;
  activeProvider: string | null;
  activeModel: string;
  acpEnabled: boolean;
  strictMemory: boolean;
}

export interface DashboardConnectivity {
  status: string;
  endpoint: string;
  host: string;
  dnsAddresses: string[];
  probeStatus: string;
  probeStatusCode: number | null;
  fakeIpDetected: boolean;
  proxyEnvDetected: boolean;
  recommendation: string | null;
}

export interface DashboardConfigSnapshot {
  activeProvider: string | null;
  lastProvider: string | null;
  model: string;
  endpoint: string;
  apiKeyConfigured: boolean;
  apiKeyMasked: string | null;
  personality: string;
  promptMode: string;
  promptAddendumConfigured: boolean;
  memoryProfile: string;
  memorySystem: string;
  sqlitePath: string;
  fileRoot: string;
  slidingWindow: number;
  summaryMaxChars: number;
}

export interface DashboardToolItem {
  id: string;
  enabled: boolean;
  detail: string;
}

export interface DashboardTools {
  approvalMode: string;
  shellDefaultMode: string;
  shellAllowCount: number;
  shellDenyCount: number;
  items: DashboardToolItem[];
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

  async loadRuntime(): Promise<DashboardRuntime> {
    const response = await apiGet<ApiEnvelope<DashboardRuntime>>(
      "/api/dashboard/runtime",
    );
    return response.data;
  },

  async loadConnectivity(): Promise<DashboardConnectivity> {
    const response = await apiGet<ApiEnvelope<DashboardConnectivity>>(
      "/api/dashboard/connectivity",
    );
    return response.data;
  },

  async loadConfig(): Promise<DashboardConfigSnapshot> {
    const response = await apiGet<ApiEnvelope<DashboardConfigSnapshot>>(
      "/api/dashboard/config",
    );
    return response.data;
  },

  async loadTools(): Promise<DashboardTools> {
    const response = await apiGet<ApiEnvelope<DashboardTools>>(
      "/api/dashboard/tools",
    );
    return response.data;
  },
};
