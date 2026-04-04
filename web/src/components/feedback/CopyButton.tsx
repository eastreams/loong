import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";

interface CopyButtonProps {
  text: string;
  className?: string;
  title?: string;
}

export function CopyButton({ text, className, title }: CopyButtonProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) {
      return;
    }

    const timer = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timer);
  }, [copied]);

  return (
    <>
      <style>
        {`
          @keyframes toastFadeInOut {
            0% { opacity: 0; transform: translate(-50%, calc(-50% + 8px)) scale(0.98); }
            12% { opacity: 1; transform: translate(-50%, -50%) scale(1); }
            88% { opacity: 1; transform: translate(-50%, -50%) scale(1); }
            100% { opacity: 0; transform: translate(-50%, calc(-50% - 8px)) scale(0.98); }
          }
        `}
      </style>
      <button
        type="button"
        className={className}
        title={title}
        onClick={() => {
          void navigator.clipboard.writeText(text);
          setCopied(true);
        }}
      >
        {copied ? <Check size={14} /> : <Copy size={14} />}
      </button>
      {copied && typeof document !== "undefined"
        ? createPortal(
            <div
              style={{
                position: "fixed",
                top: "50%",
                left: "50%",
                transform: "translate(-50%, -50%)",
                backgroundColor: "rgba(20, 18, 15, 0.82)",
                backdropFilter: "blur(10px)",
                WebkitBackdropFilter: "blur(10px)",
                color: "#fff",
                padding: "0.65rem 1rem",
                borderRadius: "999px",
                fontSize: "0.92rem",
                fontWeight: 600,
                letterSpacing: "0.01em",
                whiteSpace: "nowrap",
                pointerEvents: "none",
                animation: "toastFadeInOut 2s ease-in-out forwards",
                boxShadow: "0 18px 40px rgba(0, 0, 0, 0.16)",
                zIndex: 10000,
              }}
            >
              {t("chat.actions.copied", "复制成功")}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
