import { Suspense, useRef, type ReactNode } from "react";
import { useLocation, useOutlet } from "react-router-dom";
import NavBar from "../../components/layout/NavBar";
import { OnboardingStatusPanel } from "../../features/onboarding/components/OnboardingStatusPanel";
import { useWebConnection } from "../../hooks/useWebConnection";

const KEEP_ALIVE_ROUTE_PATHS = new Set(["/chat", "/dashboard"]);

export default function RootLayout() {
  const { onboardingBlocked } = useWebConnection();
  const location = useLocation();
  const outlet = useOutlet();
  const cachedOutletsRef = useRef<Map<string, ReactNode>>(new Map());
  const isKeepAliveRoute = KEEP_ALIVE_ROUTE_PATHS.has(location.pathname);

  if (
    !onboardingBlocked &&
    outlet &&
    isKeepAliveRoute &&
    !cachedOutletsRef.current.has(location.pathname)
  ) {
    cachedOutletsRef.current.set(location.pathname, outlet);
  }

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
        <main className="page-scroll">
          {onboardingBlocked ? (
            <OnboardingStatusPanel />
          ) : (
            <Suspense fallback={<div className="workspace-stage" aria-busy="true" />}>
              <div className="workspace-stage">
                {isKeepAliveRoute
                  ? Array.from(cachedOutletsRef.current.entries()).map(([path, element]) => (
                      <div
                        key={path}
                        className="workspace-stage-route"
                        hidden={path !== location.pathname}
                        aria-hidden={path !== location.pathname}
                      >
                        {element}
                      </div>
                    ))
                  : outlet}
              </div>
            </Suspense>
          )}
        </main>
      </div>
    </div>
  );
}
