import type { PropsWithChildren, ReactNode } from "react";

interface PanelProps extends PropsWithChildren {
  eyebrow?: string;
  title: string;
  aside?: ReactNode;
  className?: string;
  hideHeader?: boolean;
}

export function Panel({ eyebrow, title, aside, className, hideHeader, children }: PanelProps) {
  return (
    <section className={className ? `panel ${className}` : "panel"}>
      {hideHeader ? null : (
        <header className="panel-header">
          <div>
            {eyebrow ? <div className="panel-eyebrow">{eyebrow}</div> : null}
            <h2 className="panel-title">{title}</h2>
          </div>
          {aside ? <div>{aside}</div> : null}
        </header>
      )}
      <div className="panel-body">{children}</div>
    </section>
  );
}
