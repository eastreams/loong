import type { PropsWithChildren } from "react";
import NavBar from "../../components/layout/NavBar";

export default function RootLayout({ children }: PropsWithChildren) {
  return (
    <div className="app-shell">
      <div className="background-grid" aria-hidden="true" />
      <div className="background-glow background-glow-left" aria-hidden="true" />
      <div className="background-glow background-glow-right" aria-hidden="true" />
      <div className="app-frame">
        <NavBar />
        <main className="page-scroll">{children}</main>
      </div>
    </div>
  );
}
