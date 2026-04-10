import { apiGetData, type ApiRequestOptions } from "../../../lib/api/client";
import type { ProviderCatalogItem } from "../../onboarding/provider/providerCatalog";

const DASHBOARD_READ_TIMEOUT_MS = 15_000;

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
  providerBaseUrl: string;
  providerEndpointExplicit: boolean;
  endpoint: string;
  apiKeyConfigured: boolean;
  apiKeyMasked: string | null;
  personality: string;
  promptMode: string;
  promptAddendumConfigured: boolean;
  promptAddendum: string;
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
  source: string;
  capabilityState: string;
  detail: string;
}

export interface DashboardTools {
  approvalMode: string;
  shellDefaultMode: string;
  shellAllowCount: number;
  shellDenyCount: number;
  items: DashboardToolItem[];
}

export interface DashboardDebugConsole {
  generatedAt: string;
  command: string;
  blocks: DashboardDebugConsoleBlock[];
}

export interface DashboardDebugConsoleBlock {
  id: string;
  kind: string;
  startedAt: string;
  header: string;
  lines: string[];
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

interface ProviderCatalogResponse {
  items: ProviderCatalogItem[];
}

interface DashboardProvidersResponse {
  activeProvider: string | null;
  items: DashboardProviderItem[];
}

function withDefaultTimeout(request?: ApiRequestOptions): ApiRequestOptions {
  return {
    ...request,
    timeoutMs: request?.timeoutMs ?? DASHBOARD_READ_TIMEOUT_MS,
  };
}

export const dashboardApi = {
  async loadSummary(request?: ApiRequestOptions): Promise<DashboardSummary> {
    return apiGetData<DashboardSummary>(
      "/api/dashboard/summary",
      withDefaultTimeout(request),
    );
  },

  async loadProviders(request?: ApiRequestOptions): Promise<DashboardProvidersResponse> {
    return apiGetData<DashboardProvidersResponse>(
      "/api/dashboard/providers",
      withDefaultTimeout(request),
    );
  },

  async loadProviderCatalog(request?: ApiRequestOptions): Promise<ProviderCatalogItem[]> {
    const response = await apiGetData<ProviderCatalogResponse>(
      "/api/providers/catalog",
      withDefaultTimeout(request),
    );
    return response.items;
  },

  async loadRuntime(request?: ApiRequestOptions): Promise<DashboardRuntime> {
    return apiGetData<DashboardRuntime>(
      "/api/dashboard/runtime",
      withDefaultTimeout(request),
    );
  },

  async loadConnectivity(request?: ApiRequestOptions): Promise<DashboardConnectivity> {
    return apiGetData<DashboardConnectivity>(
      "/api/dashboard/connectivity",
      withDefaultTimeout(request),
    );
  },

  async loadConfig(request?: ApiRequestOptions): Promise<DashboardConfigSnapshot> {
    return apiGetData<DashboardConfigSnapshot>(
      "/api/dashboard/config",
      withDefaultTimeout(request),
    );
  },

  async loadTools(request?: ApiRequestOptions): Promise<DashboardTools> {
    return apiGetData<DashboardTools>(
      "/api/dashboard/tools",
      withDefaultTimeout(request),
    );
  },

  async loadDebugConsole(request?: ApiRequestOptions): Promise<DashboardDebugConsole> {
    return apiGetData<DashboardDebugConsole>(
      "/api/dashboard/debug-console",
      withDefaultTimeout(request),
    );
  },
};
