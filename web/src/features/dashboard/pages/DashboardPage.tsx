import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Panel } from "../../../components/surfaces/Panel";
import { dashboardApi, type DashboardProviderItem, type DashboardSummary } from "../api";

export default function DashboardPage() {
  const { t } = useTranslation();
  const [summary, setSummary] = useState<DashboardSummary | null>(null);
  const [providers, setProviders] = useState<DashboardProviderItem[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadDashboard() {
      setError(null);
      try {
        const [loadedSummary, loadedProviders] = await Promise.all([
          dashboardApi.loadSummary(),
          dashboardApi.loadProviders(),
        ]);
        if (!cancelled) {
          setSummary(loadedSummary);
          setProviders(loadedProviders.items);
        }
      } catch (loadError) {
        if (!cancelled) {
          setError(loadError instanceof Error ? loadError.message : "Failed to load dashboard");
        }
      }
    }

    void loadDashboard();

    return () => {
      cancelled = true;
    };
  }, []);

  const activeProvider = providers.find((provider) => provider.enabled) ?? providers[0] ?? null;
  const cards = [
    {
      key: "runtime",
      value: summary?.runtimeStatus ?? "Loading",
      detail: "Web API is reading runtime state from the local daemon.",
    },
    {
      key: "providers",
      value: summary?.activeProvider ?? "None",
      detail: activeProvider
        ? `${providers.length} configured profiles, active model ${activeProvider.model}`
        : "No provider profiles loaded yet.",
    },
    {
      key: "tools",
      value: "Read-only",
      detail: "Tool-level diagnostics will be expanded after the first API slice stabilizes.",
    },
    {
      key: "memory",
      value: summary?.memoryBackend ?? "Unknown",
      detail: `Detected ${summary?.sessionCount ?? 0} remembered chat sessions.`,
    },
    {
      key: "install",
      value: summary?.webInstallMode ?? "Unknown",
      detail: "Current web surface is still served as a separate optional install.",
    },
  ];

  return (
    <div className="page">
      <section className="hero-block">
        <div className="hero-eyebrow">{t("dashboard.eyebrow")}</div>
        <h1 className="hero-title">{t("dashboard.title")}</h1>
        <p className="hero-subtitle">{t("dashboard.subtitle")}</p>
      </section>

      <div className="dashboard-grid">
        {cards.map((card) => (
          <Panel
            key={card.key}
            eyebrow={t(`dashboard.cards.${card.key}`)}
            title={card.value}
          >
            <p className="panel-copy">{card.detail}</p>
          </Panel>
        ))}
      </div>

      {error ? <div className="empty-state dashboard-error">{error}</div> : null}

      <section className="dashboard-settings">
        <Panel
          eyebrow={t("dashboard.settings.eyebrow")}
          title={t("dashboard.settings.title")}
        >
          <div className="settings-header">
            <p className="panel-copy">{t("dashboard.settings.subtitle")}</p>
          </div>
          <form className="settings-form" onSubmit={(event) => event.preventDefault()}>
            <label className="settings-field">
              <span className="settings-label">
                {t("dashboard.settings.activeProvider")}
              </span>
              <select
                className="settings-input"
                defaultValue={activeProvider?.id ?? providers[0]?.id ?? ""}
              >
                {providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.label}
                  </option>
                ))}
              </select>
            </label>

            <label className="settings-field">
              <span className="settings-label">{t("dashboard.settings.model")}</span>
              <input
                className="settings-input"
                defaultValue={activeProvider?.model ?? ""}
              />
            </label>

            <label className="settings-field">
              <span className="settings-label">
                {t("dashboard.settings.endpoint")}
              </span>
              <input
                className="settings-input"
                defaultValue={activeProvider?.endpoint ?? ""}
              />
            </label>

            <label className="settings-field">
              <span className="settings-label">{t("dashboard.settings.apiKey")}</span>
              <input
                className="settings-input"
                type="password"
                defaultValue={activeProvider?.apiKeyMasked ?? ""}
              />
              <span className="settings-helper">
                {t("dashboard.settings.apiKeyMasked")}
              </span>
            </label>

            <div className="settings-actions">
              <button type="button" className="hero-btn hero-btn-secondary">
                {t("dashboard.settings.validate")}
              </button>
              <button type="submit" className="hero-btn hero-btn-primary">
                {t("dashboard.settings.apply")}
              </button>
            </div>

            <p className="settings-note">{t("dashboard.settings.helper")}</p>
          </form>
        </Panel>
      </section>
    </div>
  );
}
