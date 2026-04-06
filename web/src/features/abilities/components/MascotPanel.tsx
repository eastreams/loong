import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  readChatMascotEnabled,
  writeChatMascotEnabled,
} from "../../chat/mascotPreference";

export function MascotPanel() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(() => readChatMascotEnabled());

  const statusLabel = useMemo(
    () => (enabled ? t("abilities.common.enabled") : t("abilities.common.disabled")),
    [enabled, t],
  );

  function handleToggle() {
    const next = !enabled;
    setEnabled(next);
    writeChatMascotEnabled(next);
  }

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("app.nav.abilities")}</div>
        <h2>{t("abilities.mascot.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
        <header className="abilities-section-head">
          <h3>{t("abilities.mascot.settingsTitle")}</h3>
        </header>
        <div className="abilities-section-body">
          <div className="abilities-kv-list">
            <div className="abilities-kv-row">
              <span>{t("abilities.mascot.fields.display")}</span>
              <strong>{statusLabel}</strong>
            </div>
          </div>
          <div className="abilities-meta-divider" />
          <div className="abilities-mascot-controls">
            <p className="abilities-note">{t("abilities.mascot.helper")}</p>
            <button
              type="button"
              className={`abilities-action-button${enabled ? "" : " is-primary"}`}
              onClick={handleToggle}
            >
              {enabled
                ? t("abilities.mascot.actions.hide")
                : t("abilities.mascot.actions.show")}
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
