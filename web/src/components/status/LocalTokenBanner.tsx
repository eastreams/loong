import { FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWebConnection } from "../../hooks/useWebConnection";
import { resolveTokenHintEnv, resolveTokenHintPath } from "../../lib/auth/tokenHint";

export function LocalTokenBanner() {
  const { t } = useTranslation();
  const {
    status,
    authRequired,
    tokenEnv,
    tokenPath,
    saveToken,
    clearToken,
    onboardingStatus,
  } =
    useWebConnection();
  const [tokenInput, setTokenInput] = useState("");

  if (
    !authRequired ||
    status === "connected" ||
    onboardingStatus?.blockingStage === "runtime_offline"
  ) {
    return null;
  }

  const title =
    status === "unauthorized" ? t("auth.invalidTitle") : t("auth.bannerTitle");
  const resolvedTokenPath = resolveTokenHintPath(tokenPath);
  const resolvedTokenEnv = resolveTokenHintEnv(tokenEnv);
  const body =
    status === "unauthorized"
      ? t("auth.invalidBody", {
          tokenPath: resolvedTokenPath,
          tokenEnv: resolvedTokenEnv,
        })
      : t("auth.bannerBody", {
          tokenPath: resolvedTokenPath,
          tokenEnv: resolvedTokenEnv,
        });

  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const normalized = tokenInput.trim();
    if (!normalized) {
      return;
    }
    saveToken(normalized);
    setTokenInput("");
  }

  return (
    <section className="auth-banner" aria-live="polite">
      <div className="auth-banner-copy">
        <strong>{title}</strong>
        <span>{body}</span>
      </div>
      <form className="auth-banner-form" onSubmit={handleSubmit}>
        <input
          className="auth-banner-input"
          type="password"
          value={tokenInput}
          onChange={(event) => {
            setTokenInput(event.target.value);
          }}
          placeholder={t("auth.inputPlaceholder")}
        />
        <button type="submit" className="hero-btn hero-btn-primary">
          {t("auth.save")}
        </button>
        <button
          type="button"
          className="hero-btn hero-btn-secondary"
          onClick={clearToken}
        >
          {t("auth.clear")}
        </button>
      </form>
    </section>
  );
}
