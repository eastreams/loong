import { useTranslation } from "react-i18next";

export default function AbilitiesPage() {
  const { t } = useTranslation();

  return (
    <div className="page">
      <section className="hero-block">
        <div className="hero-eyebrow">{t("abilities.eyebrow")}</div>
        <h1 className="hero-title">{t("abilities.title")}</h1>
        <p className="hero-subtitle">{t("abilities.subtitle")}</p>
      </section>

      <section className="panel">
        <div className="panel-body">
          <p className="panel-copy">{t("abilities.placeholder")}</p>
        </div>
      </section>
    </div>
  );
}
