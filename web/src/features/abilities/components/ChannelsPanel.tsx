import { useTranslation } from "react-i18next";
import type { ChannelsSnapshot } from "../api";

interface ChannelsPanelProps {
  data: ChannelsSnapshot | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}

function formatSurfaceSource(
  source: string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (source) {
    case "runtime_backed":
      return t("abilities.channels.values.runtimeBacked");
    case "plugin_backed":
      return t("abilities.channels.values.pluginBacked");
    case "stub":
      return t("abilities.channels.values.stub");
    default:
      return source;
  }
}

function buildSurfaceFacts(
  data: ChannelsSnapshot["surfaces"][number],
  t: ReturnType<typeof useTranslation>["t"],
): string[] {
  return [
    t("abilities.channels.surfaceFacts.accounts", { count: data.configuredAccountCount }),
    t("abilities.channels.surfaceFacts.enabled", { count: data.enabledAccountCount }),
    t("abilities.channels.surfaceFacts.sendReady", { count: data.readySendAccountCount }),
    t("abilities.channels.surfaceFacts.serveReady", { count: data.readyServeAccountCount }),
    data.misconfiguredAccountCount > 0
      ? t("abilities.channels.surfaceFacts.misconfigured", { count: data.misconfiguredAccountCount })
      : t("abilities.channels.surfaceFacts.clean"),
    data.serviceEnabled
      ? data.serviceReady
        ? t("abilities.channels.surfaceFacts.serviceReady")
        : t("abilities.channels.surfaceFacts.serviceEnabled")
      : t("abilities.channels.surfaceFacts.serviceDisabled"),
  ];
}

export function ChannelsPanel({ data, loading, error, onRetry }: ChannelsPanelProps) {
  const { t } = useTranslation();

  return (
    <div className="abilities-content-stack">
      <section className="abilities-section-intro">
        <div className="hero-eyebrow">{t("nav.abilities")}</div>
        <h2>{t("abilities.channels.introTitle")}</h2>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-head">
          <div className="panel-title">{t("abilities.channels.summaryTitle")}</div>
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
                <span>{t("abilities.channels.fields.catalogChannels")}</span>
                <strong>{data.catalogChannelCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.configuredChannels")}</span>
                <strong>{data.configuredChannelCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.configuredAccounts")}</span>
                <strong>{data.configuredAccountCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.enabledAccounts")}</span>
                <strong>{data.enabledAccountCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.misconfiguredAccounts")}</span>
                <strong>{data.misconfiguredAccountCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.runtimeBackedChannels")}</span>
                <strong>{data.runtimeBackedChannelCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.serviceEnabledChannels")}</span>
                <strong>{data.enabledServiceChannelCount}</strong>
              </div>
              <div className="abilities-kv-row">
                <span>{t("abilities.channels.fields.serviceReadyChannels")}</span>
                <strong>{data.readyServiceChannelCount}</strong>
              </div>
            </div>
          ) : (
            <p className="abilities-note">{t("abilities.common.noData")}</p>
          )}
        </div>
      </section>

      <section className="abilities-section-block">
        <div className="abilities-section-head">
          <div className="panel-title">{t("abilities.channels.surfacesTitle")}</div>
        </div>
        <div className="abilities-section-body">
          {loading ? (
            <p className="abilities-note">{t("abilities.common.loading")}</p>
          ) : error ? (
            <p className="abilities-note">{t("abilities.common.loadFailed")}</p>
          ) : data && data.surfaces.length > 0 ? (
            <div className="abilities-entity-list">
              {data.surfaces.map((surface) => (
                <div key={surface.id} className="abilities-entity-row">
                  <div className="abilities-entity-head">
                    <strong>{surface.label}</strong>
                    <span>{formatSurfaceSource(surface.source, t)}</span>
                  </div>
                  <div className="abilities-entity-meta">
                    {buildSurfaceFacts(surface, t).join(" / ")}
                  </div>
                  {surface.defaultConfiguredAccountId ? (
                    <div className="abilities-entity-detail">
                      {t("abilities.channels.defaultAccount", {
                        accountId: surface.defaultConfiguredAccountId,
                      })}
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : (
            <p className="abilities-note">{t("abilities.channels.noSurfaces")}</p>
          )}
        </div>
      </section>
    </div>
  );
}
