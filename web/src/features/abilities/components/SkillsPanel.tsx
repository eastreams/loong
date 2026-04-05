import { useTranslation } from "react-i18next";
import type { SkillsSnapshot } from "../api";

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

export function SkillsPanel({ data, loading, error, onRetry }: SkillsPanelProps) {
  const { t } = useTranslation();
  const visibleTools = data?.visibleRuntimeTools ?? [];

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("nav.abilities")}</div>
        <h2>{t("abilities.skills.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
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
                <strong>{formatExecutionTier(data.browserCompanion.executionTier, t)}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.skills.fields.expectedVersion")}</span>
                <strong>{data.browserCompanion.expectedVersion ?? t("abilities.common.notAvailable")}</strong>
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

      <section className="abilities-section-block">
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
                <strong>{formatInventoryStatus(data.externalSkills.inventoryStatus, t)}</strong>
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
                <strong>{data.externalSkills.installRoot ?? t("abilities.common.notAvailable")}</strong>
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

      <section className="abilities-section-block">
        <div className="abilities-section-head">
          <div className="panel-title">{t("abilities.skills.visibleToolsTitle")}</div>
        </div>
        <div className="abilities-section-body">
          {loading ? (
            <p className="abilities-note">{t("abilities.common.loading")}</p>
          ) : error ? (
            <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
          ) : visibleTools.length > 0 ? (
            <div className="abilities-inline-list">
              {visibleTools.map((tool) => (
                <span key={tool} className="abilities-inline-item">
                  {tool}
                </span>
              ))}
            </div>
          ) : (
            <p className="abilities-note">{t("abilities.skills.noVisibleTools")}</p>
          )}
        </div>
      </section>
    </div>
  );
}
