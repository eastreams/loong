import { useTranslation } from "react-i18next";
import type { PersonalizationSnapshot } from "../api";

interface PersonalizationPanelProps {
  data: PersonalizationSnapshot | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}

function formatPromptState(
  promptState: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (promptState) {
    case "configured":
      return t("abilities.personalization.values.promptConfigured");
    case "suppressed":
      return t("abilities.personalization.values.promptSuppressed");
    case "deferred":
      return t("abilities.personalization.values.promptDeferred");
    case "pending":
    default:
      return t("abilities.personalization.values.promptPending");
  }
}

function formatResponseDensity(
  value: string | null,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "concise":
      return t("abilities.personalization.values.responseDensityConcise");
    case "balanced":
      return t("abilities.personalization.values.responseDensityBalanced");
    case "thorough":
      return t("abilities.personalization.values.responseDensityThorough");
    default:
      return t("abilities.common.notConfigured");
  }
}

function formatInitiativeLevel(
  value: string | null,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (value) {
    case "ask_before_acting":
      return t("abilities.personalization.values.initiativeAskBeforeActing");
    case "balanced":
      return t("abilities.personalization.values.initiativeBalanced");
    case "high_initiative":
      return t("abilities.personalization.values.initiativeHigh");
    default:
      return t("abilities.common.notConfigured");
  }
}

export function PersonalizationPanel({
  data,
  loading,
  error,
  onRetry,
}: PersonalizationPanelProps) {
  const { t } = useTranslation();

  const rows = data
    ? [
        {
          label: t("abilities.personalization.fields.preferredName"),
          value: data.preferredName ?? t("abilities.common.notConfigured"),
        },
        {
          label: t("abilities.personalization.fields.responseDensity"),
          value: formatResponseDensity(data.responseDensity, t),
        },
        {
          label: t("abilities.personalization.fields.initiativeLevel"),
          value: formatInitiativeLevel(data.initiativeLevel, t),
        },
        {
          label: t("abilities.personalization.fields.standingBoundaries"),
          value: data.standingBoundaries ?? t("abilities.common.notConfigured"),
        },
        {
          label: t("abilities.personalization.fields.locale"),
          value: data.locale ?? t("abilities.common.notConfigured"),
        },
        {
          label: t("abilities.personalization.fields.timezone"),
          value: data.timezone ?? t("abilities.common.notConfigured"),
        },
      ]
    : [];

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("nav.abilities")}</div>
        <h2>{t("abilities.personalization.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-head">
          <div className="panel-title">{t("abilities.personalization.snapshotTitle")}</div>
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
          ) : (
            <div className="abilities-kv-list">
              {rows.map((row) => (
                <div key={row.label} className="abilities-kv-row">
                  <span>{row.label}</span>
                  <strong>{row.value}</strong>
                </div>
              ))}
            </div>
          )}
        </div>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-head">
          <div className="panel-title">{t("abilities.personalization.statusTitle")}</div>
        </div>
        <div className="abilities-section-body">
          {loading ? (
            <p className="abilities-note">{t("abilities.common.loading")}</p>
          ) : error ? (
            <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
          ) : data ? (
            <div className="abilities-kv-list">
              <div className="abilities-kv-row">
                <span>{t("abilities.personalization.fields.profileState")}</span>
                <strong>
                  {data.configured
                    ? t("abilities.personalization.values.profileConfigured")
                    : t("abilities.personalization.values.profilePending")}
                </strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.personalization.fields.operatorPreferences")}</span>
                <strong>
                  {data.hasOperatorPreferences
                    ? t("abilities.personalization.values.preferencesPresent")
                    : t("abilities.personalization.values.preferencesAbsent")}
                </strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.personalization.fields.promptState")}</span>
                <strong>{formatPromptState(data.promptState, t)}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.personalization.fields.suggestions")}</span>
                <strong>
                  {data.suppressed
                    ? t("abilities.personalization.values.suggestionsSuppressed")
                    : t("abilities.personalization.values.suggestionsActive")}
                </strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.personalization.fields.updatedAt")}</span>
                <strong>{data.updatedAt ?? t("abilities.common.notAvailable")}</strong>
              </div>
            </div>
          ) : (
            <p className="abilities-note">{t("abilities.common.noData")}</p>
          )}
        </div>
      </section>

    </div>
  );
}
