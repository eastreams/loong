import { Cable } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useWebConnection } from "../../hooks/useWebConnection";

export function ConnectionBadge() {
  const { t } = useTranslation();
  const { endpoint, status } = useWebConnection();

  return (
    <div className="status-chip">
      <Cable size={14} />
      <span>{status === "connected" ? t("status.connected") : status}</span>
      <span className="status-chip-separator" />
      <span>{endpoint}</span>
    </div>
  );
}
