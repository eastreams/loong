import type { PropsWithChildren } from "react";
import NavBar from "../../components/layout/NavBar";
import { LocalTokenBanner } from "../../components/status/LocalTokenBanner";

export default function RootLayout({ children }: PropsWithChildren) {
  return (
    <div className="app-shell">
      <div className="background-grid" aria-hidden="true" />
      <div className="background-ornament background-ornament-top" aria-hidden="true" />
      <div className="background-ornament background-ornament-bottom" aria-hidden="true" />
      <div className="background-axis background-axis-horizontal" aria-hidden="true" />
      <div className="background-axis background-axis-vertical" aria-hidden="true" />
      <div className="background-glow background-glow-left" aria-hidden="true" />
      <div className="background-glow background-glow-right" aria-hidden="true" />
      <div className="app-frame">
        <NavBar />
        <LocalTokenBanner />
        <main className="page-scroll">{children}</main>
      </div>
    </div>
  );
}
