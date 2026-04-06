import type { DashboardDebugConsoleBlock } from "../api";

type DebugLineTone =
  | "plain"
  | "section"
  | "command"
  | "path"
  | "success"
  | "error"
  | "warn"
  | "muted";

function classifyDebugLine(line: string): DebugLineTone {
  const trimmed = line.trim();
  if (!trimmed) {
    return "muted";
  }
  if (trimmed.startsWith("$ ")) {
    return "command";
  }
  if (/^\[[^\]]+\]$/.test(trimmed)) {
    return "section";
  }
  if (/^\[error\]/i.test(trimmed)) {
    return "error";
  }
  if (/^\[hint\]/i.test(trimmed)) {
    return "muted";
  }
  if (/^\[detail\]/i.test(trimmed)) {
    return "plain";
  }
  if (/^\[latency\]/i.test(trimmed)) {
    return "path";
  }
  if (/^\[event\]/i.test(trimmed)) {
    return "muted";
  }
  if (/^\[web-api:noise\]/i.test(trimmed)) {
    return "muted";
  }
  if (/^\[tool\]/i.test(trimmed)) {
    if (/status=(ok|completed)/i.test(trimmed)) {
      return "success";
    }
    if (/status=(error|failed|timeout)/i.test(trimmed)) {
      return "error";
    }
    return "warn";
  }
  if (/^\[turn\]/i.test(trimmed)) {
    if (/status=(completed|success)/i.test(trimmed)) {
      return "success";
    }
    if (/status=(failed|error)/i.test(trimmed)) {
      return "error";
    }
    return "warn";
  }
  if (/\[(provider:error|web-api:err|web-dev:err)\]/i.test(trimmed)) {
    return "error";
  }
  if (/\[(runtime|config|provider|tools|web-api|web-dev|memory)\]/i.test(trimmed)) {
    return "section";
  }
  if (trimmed.startsWith("path=") || trimmed.includes(" path=")) {
    return "path";
  }
  if (
    /(turn\.failed|error|failed|unavailable|denied|transport_failure|Request failed)/i.test(
      trimmed,
    )
  ) {
    return "error";
  }
  if (/(ready|completed|outcome=ok|\bok\b|enabled=true)/i.test(trimmed)) {
    return "success";
  }
  if (/(started|pending|loading|in progress|outcome=started)/i.test(trimmed)) {
    return "warn";
  }
  return "plain";
}

interface DebugConsolePanelProps {
  command: string;
  blocks: DashboardDebugConsoleBlock[];
  error: string | null;
  emptyLabel: string;
}

export function DebugConsolePanel({
  command,
  blocks,
  error,
  emptyLabel,
}: DebugConsolePanelProps) {
  return (
    <div className="dashboard-debug-terminal" role="log" aria-live="polite">
      <div className="dashboard-debug-command">{command}</div>
      <div className="dashboard-debug-lines">
        {blocks.map((block) => (
          <section
            key={block.id}
            className={`dashboard-debug-block dashboard-debug-block-${block.kind}`}
          >
            <div className="dashboard-debug-block-header">{block.header}</div>
            <div className="dashboard-debug-block-lines">
              {block.lines.length > 0 ? (
                block.lines.map((line, index) => (
                  <div
                    key={`${block.id}-${index}-${line}`}
                    className={`dashboard-debug-line dashboard-debug-line-${classifyDebugLine(line)}`}
                  >
                    {line || "\u00A0"}
                  </div>
                ))
              ) : (
                <div className="dashboard-debug-line dashboard-debug-line-muted">
                  {block.kind === "loading" ? "\u00A0" : emptyLabel}
                </div>
              )}
            </div>
          </section>
        ))}
        {error ? (
          <section className="dashboard-debug-block dashboard-debug-block-error">
            <div className="dashboard-debug-block-header">ERROR</div>
            <div className="dashboard-debug-block-lines">
              <div className="dashboard-debug-line dashboard-debug-line-error">
                {error}
              </div>
            </div>
          </section>
        ) : null}
      </div>
    </div>
  );
}
