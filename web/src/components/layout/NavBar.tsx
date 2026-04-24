import { Languages, LogOut, MoonStar, SunMedium } from "lucide-react";
import { NavLink } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ConnectionBadge } from "../status/ConnectionBadge";
import { useTheme } from "../../hooks/useTheme";
import { useLocale } from "../../hooks/useLocale";
import { useWebConnection } from "../../hooks/useWebConnection";
import brandIcon from "../../assets/brand/icon.svg";

export default function NavBar() {
  const { t } = useTranslation();
  const { theme, toggleTheme } = useTheme();
  const { locale, toggleLocale } = useLocale();
  const { restartOnboarding } = useWebConnection();

  return (
    <header className="navbar">
      <div className="brand-block">
        <img src={brandIcon} alt="LoongClaw" className="brand-logo" />
        <div className="brand-lockup" aria-label={t("appName")}>
          <div className="brand-wordmark-row">
            <span className="brand-wordmark">LOONGCLAW</span>
            <span className="brand-suffix">web</span>
          </div>
        </div>
      </div>
      <nav className="nav-links" aria-label="Primary">
        <NavLink
          to="/chat"
          className={({ isActive }) => `nav-link${isActive ? " is-active" : ""}`}
        >
          {t("nav.chat")}
        </NavLink>
        <NavLink
          to="/dashboard"
          className={({ isActive }) => `nav-link${isActive ? " is-active" : ""}`}
        >
          {t("nav.dashboard")}
        </NavLink>
        <NavLink
          to="/abilities"
          className={({ isActive }) => `nav-link${isActive ? " is-active" : ""}`}
        >
          {t("nav.abilities")}
        </NavLink>
      </nav>
      <div className="nav-actions">
        <ConnectionBadge />
        <button
          type="button"
          className="navbar-btn"
          onClick={toggleLocale}
          aria-label={`${t("nav.language")}: ${locale}`}
          title={t("nav.language")}
        >
          <Languages size={16} />
        </button>
        <button
          type="button"
          className="navbar-btn"
          onClick={toggleTheme}
          data-mascot-action="toggle-theme"
          aria-label={`${t("nav.theme")}: ${theme}`}
          title={t("nav.theme")}
        >
          {theme === "dark" ? <SunMedium size={16} /> : <MoonStar size={16} />}
        </button>
        <button
          type="button"
          className="navbar-btn"
          onClick={restartOnboarding}
          aria-label={t("nav.restartOnboarding")}
          title={t("nav.restartOnboarding")}
        >
          <LogOut size={16} />
        </button>
      </div>
    </header>
  );
}
