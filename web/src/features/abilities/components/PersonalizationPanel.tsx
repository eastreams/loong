import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ApiRequestError } from "../../../lib/api/client";
import {
  AbilitiesSelectField,
} from "./AbilitiesSelectField";
import {
  abilitiesApi,
  type PersonalizationSnapshot,
  type PersonalizationWriteRequest,
} from "../api";

interface PersonalizationPanelProps {
  data: PersonalizationSnapshot | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
  onSaved: (data: PersonalizationSnapshot) => void;
}

interface PersonalizationFormState {
  preferredName: string;
  responseDensity: string;
  initiativeLevel: string;
  standingBoundaries: string;
  locale: string;
  timezone: string;
}

function buildFormState(data: PersonalizationSnapshot | null): PersonalizationFormState {
  return {
    preferredName: data?.preferredName ?? "",
    responseDensity: data?.responseDensity ?? "",
    initiativeLevel: data?.initiativeLevel ?? "",
    standingBoundaries: data?.standingBoundaries ?? "",
    locale: data?.locale ?? "",
    timezone: data?.timezone ?? "",
  };
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
  onSaved,
}: PersonalizationPanelProps) {
  const { t } = useTranslation();
  const [isEditing, setIsEditing] = useState(false);
  const [formState, setFormState] = useState<PersonalizationFormState>(() => buildFormState(data));
  const [savePending, setSavePending] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saveNotice, setSaveNotice] = useState<string | null>(null);
  const [openSelectId, setOpenSelectId] = useState<string | null>(null);

  useEffect(() => {
    if (!isEditing) {
      setFormState(buildFormState(data));
    }
  }, [data, isEditing]);

  useEffect(() => {
    if (!isEditing) {
      setOpenSelectId(null);
    }
  }, [isEditing]);

  const snapshotRows = useMemo(
    () =>
      data
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
        : [],
    [data, t],
  );

  const responseDensityOptions = useMemo(
    () => [
      { value: "", label: t("abilities.personalization.values.unset") },
      {
        value: "concise",
        label: t("abilities.personalization.values.responseDensityConcise"),
      },
      {
        value: "balanced",
        label: t("abilities.personalization.values.responseDensityBalanced"),
      },
      {
        value: "thorough",
        label: t("abilities.personalization.values.responseDensityThorough"),
      },
    ],
    [t],
  );

  const initiativeOptions = useMemo(
    () => [
      { value: "", label: t("abilities.personalization.values.unset") },
      {
        value: "ask_before_acting",
        label: t("abilities.personalization.values.initiativeAskBeforeActing"),
      },
      {
        value: "balanced",
        label: t("abilities.personalization.values.initiativeBalanced"),
      },
      {
        value: "high_initiative",
        label: t("abilities.personalization.values.initiativeHigh"),
      },
    ],
    [t],
  );

  async function handleSave() {
    const payload: PersonalizationWriteRequest = {
      preferredName: formState.preferredName,
      responseDensity: formState.responseDensity,
      initiativeLevel: formState.initiativeLevel,
      standingBoundaries: formState.standingBoundaries,
      locale: formState.locale,
      timezone: formState.timezone,
    };

    setSavePending(true);
    setSaveError(null);
    setSaveNotice(null);
    try {
      const saved = await abilitiesApi.savePersonalization(payload);
      onSaved(saved);
      setIsEditing(false);
      setSaveNotice(t("abilities.personalization.actions.saved"));
      setOpenSelectId(null);
    } catch (saveFailure) {
      const message =
        saveFailure instanceof ApiRequestError || saveFailure instanceof Error
          ? saveFailure.message
          : t("abilities.personalization.actions.saveFailed");
      setSaveError(message);
    } finally {
      setSavePending(false);
    }
  }

  function handleCancel() {
    setIsEditing(false);
    setSaveError(null);
    setSaveNotice(null);
    setFormState(buildFormState(data));
    setOpenSelectId(null);
  }

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("nav.abilities")}</div>
        <h2>{t("abilities.personalization.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-head abilities-section-head-with-actions">
          <div className="panel-title">{t("abilities.personalization.snapshotTitle")}</div>
          {loading || error ? null : (
            <div className="abilities-action-group">
              {isEditing ? (
                <>
                  <button
                    type="button"
                    className="abilities-action-button"
                    onClick={handleCancel}
                    disabled={savePending}
                  >
                    {t("abilities.personalization.actions.cancel")}
                  </button>
                  <button
                    type="button"
                    className="abilities-action-button is-primary"
                    onClick={() => void handleSave()}
                    disabled={savePending}
                  >
                    {savePending
                      ? t("abilities.personalization.actions.savePending")
                      : t("abilities.personalization.actions.save")}
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="abilities-action-button"
                  onClick={() => {
                    setSaveError(null);
                    setSaveNotice(null);
                    setIsEditing(true);
                  }}
                >
                  {t("abilities.personalization.actions.edit")}
                </button>
              )}
            </div>
          )}
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
          ) : isEditing ? (
            <div className="abilities-form-stack">
              <div className="abilities-form-grid">
                <div className="abilities-form-row">
                  <label className="abilities-form-label" htmlFor="abilities-preferred-name">
                    {t("abilities.personalization.fields.preferredName")}
                  </label>
                  <input
                    id="abilities-preferred-name"
                    className="abilities-form-input"
                    value={formState.preferredName}
                    onChange={(event) =>
                      setFormState((current) => ({
                        ...current,
                        preferredName: event.target.value,
                      }))
                    }
                  />
                </div>

                <AbilitiesSelectField
                  id="abilities-response-density"
                  label={t("abilities.personalization.fields.responseDensity")}
                  value={formState.responseDensity}
                  options={responseDensityOptions}
                  open={openSelectId === "responseDensity"}
                  onOpenChange={(open) => setOpenSelectId(open ? "responseDensity" : null)}
                  onChange={(value) =>
                    setFormState((current) => ({
                      ...current,
                      responseDensity: value,
                    }))
                  }
                />

                <AbilitiesSelectField
                  id="abilities-initiative-level"
                  label={t("abilities.personalization.fields.initiativeLevel")}
                  value={formState.initiativeLevel}
                  options={initiativeOptions}
                  open={openSelectId === "initiativeLevel"}
                  onOpenChange={(open) => setOpenSelectId(open ? "initiativeLevel" : null)}
                  onChange={(value) =>
                    setFormState((current) => ({
                      ...current,
                      initiativeLevel: value,
                    }))
                  }
                />

                <div className="abilities-form-row">
                  <label className="abilities-form-label" htmlFor="abilities-standing-boundaries">
                    {t("abilities.personalization.fields.standingBoundaries")}
                  </label>
                  <textarea
                    id="abilities-standing-boundaries"
                    className="abilities-form-textarea"
                    value={formState.standingBoundaries}
                    rows={4}
                    onChange={(event) =>
                      setFormState((current) => ({
                        ...current,
                        standingBoundaries: event.target.value,
                      }))
                    }
                  />
                </div>

                <div className="abilities-form-row">
                  <label className="abilities-form-label" htmlFor="abilities-locale">
                    {t("abilities.personalization.fields.locale")}
                  </label>
                  <input
                    id="abilities-locale"
                    className="abilities-form-input"
                    value={formState.locale}
                    onChange={(event) =>
                      setFormState((current) => ({
                        ...current,
                        locale: event.target.value,
                      }))
                    }
                  />
                </div>

                <div className="abilities-form-row">
                  <label className="abilities-form-label" htmlFor="abilities-timezone">
                    {t("abilities.personalization.fields.timezone")}
                  </label>
                  <input
                    id="abilities-timezone"
                    className="abilities-form-input"
                    value={formState.timezone}
                    onChange={(event) =>
                      setFormState((current) => ({
                        ...current,
                        timezone: event.target.value,
                      }))
                    }
                  />
                </div>
              </div>

              {saveError ? <p className="abilities-error">{saveError}</p> : null}
              {saveNotice ? <p className="abilities-note">{saveNotice}</p> : null}
            </div>
          ) : (
            <>
              <div className="abilities-kv-list">
                {snapshotRows.map((row) => (
                  <div key={row.label} className="abilities-kv-row">
                    <span>{row.label}</span>
                    <strong>{row.value}</strong>
                  </div>
                ))}
              </div>
              <div className="abilities-meta-divider" />
              <div className="abilities-meta-row">
                <span>{t("abilities.personalization.fields.updatedAt")}</span>
                <strong>{data?.updatedAt ?? t("abilities.common.notAvailable")}</strong>
              </div>
            </>
          )}
        </div>
      </section>
    </div>
  );
}
