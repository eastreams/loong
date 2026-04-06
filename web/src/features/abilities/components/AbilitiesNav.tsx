import { useTranslation } from "react-i18next";

export type AbilitiesSection = "personalization" | "channels" | "skills" | "mascot";

interface AbilitiesNavProps {
  activeSection: AbilitiesSection;
  onChange: (section: AbilitiesSection) => void;
}

const SECTION_ORDER: AbilitiesSection[] = ["personalization", "channels", "skills", "mascot"];

export function AbilitiesNav({ activeSection, onChange }: AbilitiesNavProps) {
  const { t } = useTranslation();

  return (
    <nav className="abilities-nav" aria-label={t("abilities.navLabel")}>
      <div className="abilities-nav-list">
        {SECTION_ORDER.map((section) => {
          const selected = section === activeSection;
          return (
            <button
              key={section}
              type="button"
              className={`abilities-nav-item${selected ? " is-active" : ""}`}
              onClick={() => onChange(section)}
              aria-pressed={selected}
            >
              <span className="abilities-nav-item-label">
                {t(`abilities.sections.${section}.title`)}
              </span>
              <span className="abilities-nav-item-copy">
                {t(`abilities.sections.${section}.summary`)}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
}
