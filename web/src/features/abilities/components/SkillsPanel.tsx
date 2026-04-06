import { useTranslation } from "react-i18next";
import type { SkillsSnapshot } from "../api";

interface SkillsPanelProps {
  data: SkillsSnapshot | null;
  loading: boolean;
  error: string | null;
  onRetry: () => void;
}

type VisibleToolSource =
  | "session"
  | "execution"
  | "browser"
  | "local"
  | "provider"
  | "delegation"
  | "approval"
  | "external"
  | "system"
  | "runtime";

interface VisibleToolPresentation {
  id: string;
  label: string;
  summary: string;
  source: string;
  sourceWeight: number;
}

interface VisibleToolDescriptor {
  labelKey: string;
  summaryKey: string;
  source: VisibleToolSource;
}

const VISIBLE_TOOL_DESCRIPTORS: Record<string, VisibleToolDescriptor> = {
  approval_request_resolve: {
    labelKey: "approvalRequestResolve",
    summaryKey: "approvalRequestResolve",
    source: "approval",
  },
  approval_request_status: {
    labelKey: "approvalRequestStatus",
    summaryKey: "approvalRequestStatus",
    source: "approval",
  },
  approval_requests_list: {
    labelKey: "approvalRequestsList",
    summaryKey: "approvalRequestsList",
    source: "approval",
  },
  "bash.exec": {
    labelKey: "bashExec",
    summaryKey: "bashExec",
    source: "execution",
  },
  "browser.click": {
    labelKey: "browserClick",
    summaryKey: "browserClick",
    source: "browser",
  },
  "browser.extract": {
    labelKey: "browserExtract",
    summaryKey: "browserExtract",
    source: "browser",
  },
  "browser.open": {
    labelKey: "browserOpen",
    summaryKey: "browserOpen",
    source: "browser",
  },
  "claw.migrate": {
    labelKey: "clawMigrate",
    summaryKey: "clawMigrate",
    source: "system",
  },
  delegate: {
    labelKey: "delegate",
    summaryKey: "delegate",
    source: "delegation",
  },
  delegate_async: {
    labelKey: "delegateAsync",
    summaryKey: "delegateAsync",
    source: "delegation",
  },
  "external_skills.policy": {
    labelKey: "externalSkillsPolicy",
    summaryKey: "externalSkillsPolicy",
    source: "external",
  },
  "file.edit": {
    labelKey: "fileEdit",
    summaryKey: "fileEdit",
    source: "local",
  },
  "file.read": {
    labelKey: "fileRead",
    summaryKey: "fileRead",
    source: "local",
  },
  "file.write": {
    labelKey: "fileWrite",
    summaryKey: "fileWrite",
    source: "local",
  },
  "provider.switch": {
    labelKey: "providerSwitch",
    summaryKey: "providerSwitch",
    source: "provider",
  },
  session_events: {
    labelKey: "sessionEvents",
    summaryKey: "sessionEvents",
    source: "session",
  },
  session_search: {
    labelKey: "sessionSearch",
    summaryKey: "sessionSearch",
    source: "session",
  },
  session_status: {
    labelKey: "sessionStatus",
    summaryKey: "sessionStatus",
    source: "session",
  },
  session_tool_policy_status: {
    labelKey: "sessionToolPolicyStatus",
    summaryKey: "sessionToolPolicyStatus",
    source: "session",
  },
  session_wait: {
    labelKey: "sessionWait",
    summaryKey: "sessionWait",
    source: "session",
  },
  sessions_history: {
    labelKey: "sessionsHistory",
    summaryKey: "sessionsHistory",
    source: "session",
  },
  sessions_list: {
    labelKey: "sessionsList",
    summaryKey: "sessionsList",
    source: "session",
  },
  "shell.exec": {
    labelKey: "shellExec",
    summaryKey: "shellExec",
    source: "execution",
  },
  "tool.invoke": {
    labelKey: "toolInvoke",
    summaryKey: "toolInvoke",
    source: "runtime",
  },
  "tool.search": {
    labelKey: "toolSearch",
    summaryKey: "toolSearch",
    source: "runtime",
  },
  "web.fetch": {
    labelKey: "webFetch",
    summaryKey: "webFetch",
    source: "browser",
  },
  "web.search": {
    labelKey: "webSearch",
    summaryKey: "webSearch",
    source: "provider",
  },
};

function inferVisibleToolSource(toolId: string): VisibleToolSource {
  if (toolId.startsWith("session_") || toolId.startsWith("sessions_")) {
    return "session";
  }
  if (toolId.startsWith("browser.") || toolId.startsWith("web.")) {
    return "browser";
  }
  if (toolId.startsWith("file.")) {
    return "local";
  }
  if (toolId.startsWith("approval_request")) {
    return "approval";
  }
  if (toolId.startsWith("delegate")) {
    return "delegation";
  }
  if (toolId.startsWith("provider.")) {
    return "provider";
  }
  if (toolId === "bash.exec" || toolId === "shell.exec") {
    return "execution";
  }
  if (toolId.startsWith("external_skills.")) {
    return "external";
  }
  if (toolId.startsWith("claw.")) {
    return "system";
  }
  return "runtime";
}

function formatVisibleToolSource(
  source: VisibleToolSource,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (source) {
    case "session":
      return t("abilities.skills.sources.session");
    case "execution":
      return t("abilities.skills.sources.execution");
    case "browser":
      return t("abilities.skills.sources.browser");
    case "local":
      return t("abilities.skills.sources.local");
    case "provider":
      return t("abilities.skills.sources.provider");
    case "delegation":
      return t("abilities.skills.sources.delegation");
    case "approval":
      return t("abilities.skills.sources.approval");
    case "external":
      return t("abilities.skills.sources.external");
    case "system":
      return t("abilities.skills.sources.system");
    default:
      return t("abilities.skills.sources.runtime");
  }
}

function sourceWeight(source: VisibleToolSource): number {
  switch (source) {
    case "session":
      return 1;
    case "execution":
      return 2;
    case "browser":
      return 3;
    case "local":
      return 4;
    case "provider":
      return 5;
    case "delegation":
      return 6;
    case "approval":
      return 7;
    case "external":
      return 8;
    case "system":
      return 9;
    default:
      return 10;
  }
}

function humanizeToolId(toolId: string): string {
  return toolId
    .split(/[._]/g)
    .filter(Boolean)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(" ");
}

function resolveVisibleToolPresentation(
  toolId: string,
  t: ReturnType<typeof useTranslation>["t"],
): VisibleToolPresentation {
  const descriptor = VISIBLE_TOOL_DESCRIPTORS[toolId];
  const source = descriptor?.source ?? inferVisibleToolSource(toolId);

  return {
    id: toolId,
    label: descriptor
      ? t(`abilities.skills.toolLabels.${descriptor.labelKey}`)
      : humanizeToolId(toolId),
    summary: descriptor
      ? t(`abilities.skills.toolSummaries.${descriptor.summaryKey}`)
      : t("abilities.skills.values.toolSummaryFallback", { tool: toolId }),
    source: formatVisibleToolSource(source, t),
    sourceWeight: sourceWeight(source),
  };
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
  const visibleTools = (data?.visibleRuntimeTools ?? [])
    .map((toolId) => resolveVisibleToolPresentation(toolId, t))
    .sort((left, right) => {
      if (left.sourceWeight !== right.sourceWeight) {
        return left.sourceWeight - right.sourceWeight;
      }
      return left.label.localeCompare(right.label);
    });

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
                      <div
                        key={tool.id}
                        className="abilities-entity-row"
                        title={tool.summary}
                        aria-label={`${tool.id}: ${tool.summary}`}
                      >
                        <div className="abilities-entity-head abilities-entity-head-inline">
                          <strong className="abilities-tool-id">{tool.id}</strong>
                          <span>{tool.source}</span>
                        </div>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="abilities-note">{t("abilities.skills.noVisibleTools")}</p>
                )}
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
