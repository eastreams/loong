import type { PropsWithChildren } from "react";
import { ThemeProvider } from "../contexts/ThemeContext";
import { WebSessionProvider } from "../contexts/WebSessionContext";

export function AppProviders({ children }: PropsWithChildren) {
  return (
    <ThemeProvider>
      <WebSessionProvider>{children}</WebSessionProvider>
    </ThemeProvider>
  );
}
