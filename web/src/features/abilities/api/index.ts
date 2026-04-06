import {
  apiGetData,
  apiPostData,
  type ApiRequestOptions,
} from "../../../lib/api/client";

const ABILITIES_READ_TIMEOUT_MS = 15_000;

export interface PersonalizationSnapshot {
  configured: boolean;
  hasOperatorPreferences: boolean;
  suppressed: boolean;
  promptState: string;
  updatedAt: string | null;
  preferredName: string | null;
  responseDensity: string | null;
  initiativeLevel: string | null;
  standingBoundaries: string | null;
  locale: string | null;
  timezone: string | null;
}

export interface PersonalizationWriteRequest {
  preferredName: string;
  responseDensity: string;
  initiativeLevel: string;
  standingBoundaries: string;
  locale: string;
  timezone: string;
  promptState?: string;
}

export interface ChannelSurfaceSnapshot {
  id: string;
  label: string;
  source: string;
  configuredAccountCount: number;
  enabledAccountCount: number;
  misconfiguredAccountCount: number;
  readySendAccountCount: number;
  readyServeAccountCount: number;
  defaultConfiguredAccountId: string | null;
  serviceEnabled: boolean;
  serviceReady: boolean;
}

export interface ChannelsSnapshot {
  catalogChannelCount: number;
  configuredChannelCount: number;
  configuredAccountCount: number;
  enabledAccountCount: number;
  misconfiguredAccountCount: number;
  runtimeBackedChannelCount: number;
  enabledServiceChannelCount: number;
  readyServiceChannelCount: number;
  surfaces: ChannelSurfaceSnapshot[];
}

export interface BrowserCompanionSnapshot {
  enabled: boolean;
  ready: boolean;
  commandConfigured: boolean;
  expectedVersion: string | null;
  executionTier: string;
  timeoutSeconds: number;
}

export interface ExternalSkillsSnapshot {
  enabled: boolean;
  overrideActive: boolean;
  inventoryStatus: string;
  inventoryError: string | null;
  requireDownloadApproval: boolean;
  autoExposeInstalled: boolean;
  installRoot: string | null;
  allowedDomainCount: number;
  blockedDomainCount: number;
  resolvedSkillCount: number;
  shadowedSkillCount: number;
}

export interface SkillsSnapshot {
  visibleRuntimeToolCount: number;
  visibleRuntimeTools: string[];
  browserCompanion: BrowserCompanionSnapshot;
  externalSkills: ExternalSkillsSnapshot;
}

function withDefaultTimeout(request?: ApiRequestOptions): ApiRequestOptions {
  return {
    ...request,
    timeoutMs: request?.timeoutMs ?? ABILITIES_READ_TIMEOUT_MS,
  };
}

export const abilitiesApi = {
  async loadPersonalization(request?: ApiRequestOptions): Promise<PersonalizationSnapshot> {
    return apiGetData<PersonalizationSnapshot>(
      "/api/abilities/personalization",
      withDefaultTimeout(request),
    );
  },

  async savePersonalization(
    body: PersonalizationWriteRequest,
    request?: ApiRequestOptions,
  ): Promise<PersonalizationSnapshot> {
    return apiPostData<PersonalizationSnapshot, PersonalizationWriteRequest>(
      "/api/abilities/personalization",
      body,
      withDefaultTimeout(request),
    );
  },

  async loadChannels(request?: ApiRequestOptions): Promise<ChannelsSnapshot> {
    return apiGetData<ChannelsSnapshot>(
      "/api/abilities/channels",
      withDefaultTimeout(request),
    );
  },

  async loadSkills(request?: ApiRequestOptions): Promise<SkillsSnapshot> {
    return apiGetData<SkillsSnapshot>(
      "/api/abilities/skills",
      withDefaultTimeout(request),
    );
  },
};
