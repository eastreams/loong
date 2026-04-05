import { useState } from "react";
import "../../../styles/abilities.css";
import { useWebConnection } from "../../../hooks/useWebConnection";
import { ChannelsPanel } from "../components/ChannelsPanel";
import { PersonalizationPanel } from "../components/PersonalizationPanel";
import { SkillsPanel } from "../components/SkillsPanel";
import { AbilitiesNav, type AbilitiesSection } from "../components/AbilitiesNav";
import { useAbilitiesData } from "../hooks/useAbilitiesData";

export default function AbilitiesPage() {
  const connection = useWebConnection();
  const { canAccessProtectedApi, authRevision, markUnauthorized } = connection;
  const [activeSection, setActiveSection] = useState<AbilitiesSection>("personalization");
  const { personalization, channels, skills, reloadSection } = useAbilitiesData({
    activeSection,
    canAccessProtectedApi,
    authRevision,
    markUnauthorized,
  });

  const renderSection = () => {
    if (activeSection === "personalization") {
      return (
        <PersonalizationPanel
          data={personalization.data}
          loading={personalization.loading}
          error={personalization.error}
          onRetry={() => reloadSection("personalization")}
        />
      );
    }

    if (activeSection === "channels") {
      return (
        <ChannelsPanel
          data={channels.data}
          loading={channels.loading}
          error={channels.error}
          onRetry={() => reloadSection("channels")}
        />
      );
    }

    return (
      <SkillsPanel
        data={skills.data}
        loading={skills.loading}
        error={skills.error}
        onRetry={() => reloadSection("skills")}
      />
    );
  };

  return (
    <div className="page page-abilities">
      <div className="abilities-shell">
        <aside className="abilities-sidebar">
          <AbilitiesNav activeSection={activeSection} onChange={setActiveSection} />
        </aside>
        <section className="abilities-main">
          {renderSection()}
        </section>
      </div>
    </div>
  );
}
