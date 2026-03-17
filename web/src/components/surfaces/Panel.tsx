import type { PropsWithChildren, ReactNode } from "react";

interface PanelProps extends PropsWithChildren {
  eyebrow?: string;
  title: string;
  aside?: ReactNode;
}

export function Panel({ eyebrow, title, aside, children }: PanelProps) {
  return (
    <section className="panel">
      <header className="panel-header">
        <div>
          {eyebrow ? <div className="panel-eyebrow">{eyebrow}</div> : null}
          <h2 className="panel-title">{title}</h2>
        </div>
        {aside ? <div>{aside}</div> : null}
      </header>
      <div className="panel-body">{children}</div>
    </section>
  );
}
