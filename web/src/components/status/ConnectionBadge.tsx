import { Cable } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useWebConnection } from "../../hooks/useWebConnection";

export function ConnectionBadge() {
  const { t } = useTranslation();
  const { endpoint, status } = useWebConnection();
  const statusLabel =
    status === "connected"
      ? t("status.connected")
      : status === "auth_required"
        ? t("status.authRequired")
        : t("status.unauthorized");

  return (
    <div className={`status-chip status-chip-${status}`}>
      <Cable size={14} />
      <span>{statusLabel}</span>
      <span className="status-chip-separator" />
      <span>{endpoint}</span>
    </div>
  );
}
