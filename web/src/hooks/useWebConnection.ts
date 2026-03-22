import { useContext } from "react";
import { WebSessionContext } from "../contexts/WebSessionContext";

export function useWebConnection() {
  const context = useContext(WebSessionContext);
  if (!context) {
    throw new Error("useWebConnection must be used within WebSessionProvider");
  }
  return context;
}
