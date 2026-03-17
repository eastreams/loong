import { createContext, useMemo, type PropsWithChildren } from "react";
import { getApiBaseUrl } from "../lib/config/env";

export interface WebSessionContextValue {
  endpoint: string;
  status: "connected";
}

export const WebSessionContext = createContext<WebSessionContextValue | null>(null);

export function WebSessionProvider({ children }: PropsWithChildren) {
  const value = useMemo<WebSessionContextValue>(
    () => ({
      endpoint: getApiBaseUrl(),
      status: "connected",
    }),
    [],
  );

  return (
    <WebSessionContext.Provider value={value}>
      {children}
    </WebSessionContext.Provider>
  );
}
