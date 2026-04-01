import { createContext, type PropsWithChildren } from "react";
import { useWebSessionManager } from "../hooks/useWebSessionManager";
import type { OnboardingStatus } from "../features/onboarding/api";

export interface WebSessionContextValue {
  endpoint: string;
  status: "connected" | "auth_required" | "unauthorized";
  authRequired: boolean;
  canAccessProtectedApi: boolean;
  onboardingLoading: boolean;
  onboardingStatus: OnboardingStatus | null;
  onboardingBlocked: boolean;
  onboardingValidationSatisfied: boolean;
  acknowledgeOnboarding: () => void;
  markOnboardingValidated: () => void;
  acceptValidatedOnboardingStatus: (status: OnboardingStatus) => void;
  clearOnboardingValidation: () => void;
  refreshOnboardingStatus: () => void;
  restartOnboarding: () => void;
  autoPairingInProgress: boolean;
  tokenPath: string | null;
  tokenEnv: string | null;
  authMode: string | null;
  authRevision: number;
  saveToken: (token: string) => void;
  clearToken: () => void;
  markUnauthorized: () => void;
}

export const WebSessionContext = createContext<WebSessionContextValue | null>(null);

export function WebSessionProvider({ children }: PropsWithChildren) {
  const value = useWebSessionManager();

  return (
    <WebSessionContext.Provider value={value}>
      {children}
    </WebSessionContext.Provider>
  );
}