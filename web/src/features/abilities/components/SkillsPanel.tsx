import { useTranslation } from "react-i18next";
import type {
  HiddenToolSurfaceSnapshot,
  SkillsSnapshot,
  VisibleToolSnapshot,
} from "../api";

interface SkillsPanelProps {
  data: SkillsSnapshot | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}

function formatInventoryStatus(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "ok":
      return t("abilities.skills.values.inventoryOk");
    case "missing":
      return t("abilities.skills.values.inventoryMissing");
    case "error":
      return t("abilities.skills.values.inventoryError");
    case "disabled":
      return t("abilities.skills.values.inventoryDisabled");
    default:
      return value;
  }
}

function formatExecutionTier(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "local":
      return t("abilities.skills.values.executionLocal");
    case "browser_companion":
      return t("abilities.skills.values.executionBrowserCompanion");
    default:
      return value;
  }
}

function formatApprovalMode(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "disabled":
      return t("abilities.skills.values.approvalDisabled");
    case "medium_balanced":
      return t("abilities.skills.values.approvalMediumBalanced");
    case "strict":
      return t("abilities.skills.values.approvalStrict");
    default:
      return value;
  }
}

function formatAutonomyProfile(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "discovery_only":
      return t("abilities.skills.values.autonomyDiscoveryOnly");
    case "guided_acquisition":
      return t("abilities.skills.values.autonomyGuidedAcquisition");
    case "bounded_autonomous":
      return t("abilities.skills.values.autonomyBoundedAutonomous");
    default:
      return value;
  }
}

function formatConsentDefaultMode(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "prompt":
      return t("abilities.skills.values.consentPrompt");
    case "auto":
      return t("abilities.skills.values.consentAuto");
    case "full":
      return t("abilities.skills.values.consentFull");
    default:
      return value;
  }
}

function formatToolExposure(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "direct":
      return t("abilities.skills.values.exposureDirect");
    case "discoverable":
      return t("abilities.skills.values.exposureDiscoverable");
    case "hidden":
      return t("abilities.skills.values.exposureHidden");
    default:
      return value;
  }
}

function formatExecutionKind(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "core":
      return t("abilities.skills.values.executionKindCore");
    case "app":
      return t("abilities.skills.values.executionKindApp");
    default:
      return value;
  }
}

function formatCapabilityActionClass(
  value: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "discover":
      return t("abilities.skills.values.capabilityDiscover");
    case "execute_existing":
      return t("abilities.skills.values.capabilityExecuteExisting");
    case "capability_fetch":
      return t("abilities.skills.values.capabilityFetch");
    case "capability_install":
      return t("abilities.skills.values.capabilityInstall");
    case "capability_load":
      return t("abilities.skills.values.capabilityLoad");
    case "runtime_switch":
      return t("abilities.skills.values.capabilityRuntimeSwitch");
    case "topology_expand":
      return t("abilities.skills.values.capabilityTopologyExpand");
    case "policy_mutation":
      return t("abilities.skills.values.capabilityPolicyMutation");
    case "session_mutation":
      return t("abilities.skills.values.capabilitySessionMutation");
    default:
      return value;
  }
}

function buildHiddenSurfaceSummary(
  surface: HiddenToolSurfaceSnapshot,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  if (surface.visibleToolNames.length === 0) {
    return t("abilities.common.notAvailable");
  }

  return surface.visibleToolNames.join(", ");
}

function sortVisibleTools(
  left: VisibleToolSnapshot,
  right: VisibleToolSnapshot,
): number {
  const leftSurface = left.surfaceId ?? "";
  const rightSurface = right.surfaceId ?? "";

  if (leftSurface !== rightSurface) {
    return leftSurface.localeCompare(rightSurface);
  }

  return left.displayName.localeCompare(right.displayName);
}

function sortHiddenSurfaces(
  left: HiddenToolSurfaceSnapshot,
  right: HiddenToolSurfaceSnapshot,
): number {
  return left.surfaceId.localeCompare(right.surfaceId);
}

export function SkillsPanel({ data, loading, error, onRetry }: SkillsPanelProps) {
  const { t } = useTranslation();
  const visibleTools = [...(data?.visibleRuntimeCatalog ?? [])].sort(sortVisibleTools);
  const hiddenSurfaces = [...(data?.hiddenToolSurfaces ?? [])].sort(sortHiddenSurfaces);

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("nav.abilities")}</div>
        <h2>{t("abilities.skills.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-body">
          <div className="abilities-skills-split">
            <div className="abilities-skills-pane abilities-skills-pane-summary">
              <section className="abilities-section-block abilities-section-block-nested">
                <div className="abilities-section-head">
                  <div className="panel-title">{t("abilities.skills.runtimeTitle")}</div>
                </div>
                <div className="abilities-section-body">
                  {loading ? (
                    <p className="abilities-note">{t("abilities.common.loading")}</p>
                  ) : error ? (
                    <div className="abilities-feedback-block">
                      <p className="abilities-error">{error}</p>
                      <button type="button" className="abilities-inline-action" onClick={onRetry}>
                        {t("abilities.common.retry")}
                      </button>
                    </div>
                  ) : data ? (
                    <div className="abilities-kv-list">
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.visibleRuntimeTools")}</span>
                        <strong>{data.visibleRuntimeToolCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.visibleRuntimeDirectTools")}</span>
                        <strong>{data.visibleRuntimeDirectToolCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.hiddenToolCount")}</span>
                        <strong>{data.hiddenToolCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.approvalMode")}</span>
                        <strong>{formatApprovalMode(data.approvalMode, t)}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.autonomyProfile")}</span>
                        <strong>{formatAutonomyProfile(data.autonomyProfile, t)}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.consentDefaultMode")}</span>
                        <strong>
                          {formatConsentDefaultMode(data.consentDefaultMode, t)}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.sessionsAllowMutation")}</span>
                        <strong>
                          {data.sessionsAllowMutation
                            ? t("abilities.common.enabled")
                            : t("abilities.common.disabled")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.browserCompanionEnabled")}</span>
                        <strong>
                          {data.browserCompanion.enabled
                            ? t("abilities.common.enabled")
                            : t("abilities.common.disabled")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.browserCompanionReady")}</span>
                        <strong>
                          {data.browserCompanion.ready
                            ? t("abilities.skills.values.ready")
                            : t("abilities.skills.values.notReady")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.commandConfigured")}</span>
                        <strong>
                          {data.browserCompanion.commandConfigured
                            ? t("abilities.common.yes")
                            : t("abilities.common.no")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.executionTier")}</span>
                        <strong>
                          {formatExecutionTier(data.browserCompanion.executionTier, t)}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.expectedVersion")}</span>
                        <strong>
                          {data.browserCompanion.expectedVersion ??
                            t("abilities.common.notAvailable")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.timeoutSeconds")}</span>
                        <strong>
                          {t("abilities.skills.values.timeoutSeconds", {
                            count: data.browserCompanion.timeoutSeconds,
                          })}
                        </strong>
                      </div>
                    </div>
                  ) : (
                    <p className="abilities-note">{t("abilities.common.noData")}</p>
                  )}
                </div>
              </section>

              <section className="abilities-section-block abilities-section-block-nested">
                <div className="abilities-section-head">
                  <div className="panel-title">{t("abilities.skills.externalTitle")}</div>
                </div>
                <div className="abilities-section-body">
                  {loading ? (
                    <p className="abilities-note">{t("abilities.common.loading")}</p>
                  ) : error ? (
                    <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
                  ) : data ? (
                    <div className="abilities-kv-list">
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.externalSkillsEnabled")}</span>
                        <strong>
                          {data.externalSkills.enabled
                            ? t("abilities.common.enabled")
                            : t("abilities.common.disabled")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.inventoryStatus")}</span>
                        <strong>
                          {formatInventoryStatus(data.externalSkills.inventoryStatus, t)}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.resolvedSkillCount")}</span>
                        <strong>{data.externalSkills.resolvedSkillCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.shadowedSkillCount")}</span>
                        <strong>{data.externalSkills.shadowedSkillCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.requireDownloadApproval")}</span>
                        <strong>
                          {data.externalSkills.requireDownloadApproval
                            ? t("abilities.common.yes")
                            : t("abilities.common.no")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.autoExposeInstalled")}</span>
                        <strong>
                          {data.externalSkills.autoExposeInstalled
                            ? t("abilities.common.enabled")
                            : t("abilities.common.disabled")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.overrideActive")}</span>
                        <strong>
                          {data.externalSkills.overrideActive
                            ? t("abilities.common.enabled")
                            : t("abilities.common.disabled")}
                        </strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.allowedDomainCount")}</span>
                        <strong>{data.externalSkills.allowedDomainCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.blockedDomainCount")}</span>
                        <strong>{data.externalSkills.blockedDomainCount}</strong>
                      </div>
                      <div className="abilities-kv-row">
                        <span>{t("abilities.skills.fields.installRoot")}</span>
                        <strong>
                          {data.externalSkills.installRoot ??
                            t("abilities.common.notAvailable")}
                        </strong>
                      </div>
                      {data.externalSkills.inventoryError ? (
                        <div className="abilities-kv-row">
                          <span>{t("abilities.skills.fields.inventoryError")}</span>
                          <strong>{data.externalSkills.inventoryError}</strong>
                        </div>
                      ) : null}
                    </div>
                  ) : (
                    <p className="abilities-note">{t("abilities.common.noData")}</p>
                  )}
                </div>
              </section>
            </div>

            <div className="abilities-skills-pane abilities-skills-pane-tools">
              <div className="abilities-skills-tools-head">
                <div className="panel-title">{t("abilities.skills.visibleToolsTitle")}</div>
              </div>
              <div className="abilities-skills-scroll">
                {loading ? (
                  <p className="abilities-note">{t("abilities.common.loading")}</p>
                ) : error ? (
                  <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
                ) : visibleTools.length > 0 ? (
                  <div className="abilities-entity-list">
                    {visibleTools.map((tool) => (
                      <div key={tool.visibleName} className="abilities-entity-row">
                        <div className="abilities-entity-head">
                          <div className="abilities-entity-title">
                            <strong>{tool.displayName}</strong>
                          </div>
                          <span>{formatToolExposure(tool.exposure, t)}</span>
                        </div>
                        <div className="abilities-entity-meta">{tool.summary}</div>
                        <div className="abilities-inline-list">
                          <span className="abilities-inline-item">
                            {t("abilities.skills.fields.visibleName")}:{" "}
                            <span className="abilities-tool-id">{tool.visibleName}</span>
                          </span>
                          {tool.canonicalName !== tool.visibleName ? (
                            <span className="abilities-inline-item">
                              {t("abilities.skills.fields.canonicalName")}:{" "}
                              <span className="abilities-tool-id">{tool.canonicalName}</span>
                            </span>
                          ) : null}
                          <span className="abilities-inline-item">
                            {t("abilities.skills.fields.surfaceId")}:{" "}
                            {tool.surfaceId ?? t("abilities.common.notAvailable")}
                          </span>
                          <span className="abilities-inline-item">
                            {t("abilities.skills.fields.executionKind")}:{" "}
                            {formatExecutionKind(tool.executionKind, t)}
                          </span>
                          <span className="abilities-inline-item">
                            {t("abilities.skills.fields.capabilityActionClass")}:{" "}
                            {formatCapabilityActionClass(tool.capabilityActionClass, t)}
                          </span>
                        </div>
                        {tool.usageGuidance ? (
                          <div className="abilities-entity-detail">{tool.usageGuidance}</div>
                        ) : null}
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="abilities-note">{t("abilities.skills.noVisibleTools")}</p>
                )}

                <section className="abilities-section-block abilities-section-block-nested">
                  <div className="abilities-section-head">
                    <div className="panel-title">{t("abilities.skills.hiddenToolsTitle")}</div>
                  </div>
                  <div className="abilities-section-body">
                    {loading ? (
                      <p className="abilities-note">{t("abilities.common.loading")}</p>
                    ) : error ? (
                      <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
                    ) : hiddenSurfaces.length > 0 ? (
                      <div className="abilities-entity-list">
                        {hiddenSurfaces.map((surface) => (
                          <div key={surface.surfaceId} className="abilities-entity-row">
                            <div className="abilities-entity-head">
                              <div className="abilities-entity-title">
                                <strong>{surface.surfaceId}</strong>
                              </div>
                              <span>
                                {t("abilities.skills.values.hiddenSurfaceTools", {
                                  count: surface.toolCount,
                                })}
                              </span>
                            </div>
                            <div className="abilities-entity-meta">
                              {surface.usageGuidance}
                            </div>
                            <div className="abilities-entity-detail">
                              {t("abilities.skills.fields.visibleNames")}:{" "}
                              {buildHiddenSurfaceSummary(surface, t)}
                            </div>
                          </div>
                        ))}
                      </div>
                    ) : (
                      <p className="abilities-note">
                        {t("abilities.skills.noHiddenTools")}
                      </p>
                    )}
                  </div>
                </section>
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
