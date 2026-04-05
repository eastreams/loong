import { useEffect, useState } from "react";
import { ApiRequestError } from "../../../lib/api/client";
import type {
  ChannelsSnapshot,
  PersonalizationSnapshot,
  SkillsSnapshot,
} from "../api";
import { abilitiesApi } from "../api";
import type { AbilitiesSection } from "../components/AbilitiesNav";

interface SectionState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  loaded: boolean;
}

function idleState<T>(): SectionState<T> {
  return {
    data: null,
    error: null,
    loading: false,
    loaded: false,
  };
}

interface UseAbilitiesDataOptions {
  activeSection: AbilitiesSection;
  canAccessProtectedApi: boolean;
  authRevision: number;
  markUnauthorized: () => void;
}

export function useAbilitiesData({
  activeSection,
  canAccessProtectedApi,
  authRevision,
  markUnauthorized,
}: UseAbilitiesDataOptions) {
  const [personalization, setPersonalization] = useState<SectionState<PersonalizationSnapshot>>(
    () => idleState(),
  );
  const [channels, setChannels] = useState<SectionState<ChannelsSnapshot>>(() => idleState());
  const [skills, setSkills] = useState<SectionState<SkillsSnapshot>>(() => idleState());

  useEffect(() => {
    setPersonalization(idleState());
    setChannels(idleState());
    setSkills(idleState());
  }, [authRevision]);

  useEffect(() => {
    if (!canAccessProtectedApi) {
      if (activeSection === "personalization") {
        setPersonalization((current) =>
          current.loading
            ? { ...current, loading: false, error: current.error ?? null }
            : current,
        );
      } else if (activeSection === "channels") {
        setChannels((current) =>
          current.loading ? { ...current, loading: false, error: current.error ?? null } : current,
        );
      } else {
        setSkills((current) =>
          current.loading ? { ...current, loading: false, error: current.error ?? null } : current,
        );
      }
      return;
    }

    let cancelled = false;
    const controller = new AbortController();

    async function loadPersonalization() {
      if (personalization.loaded || personalization.loading) {
        return;
      }
      setPersonalization((current) => ({ ...current, loading: true, error: null }));
      try {
        const data = await abilitiesApi.loadPersonalization({ signal: controller.signal });
        if (!cancelled) {
          setPersonalization({
            data,
            error: null,
            loading: false,
            loaded: true,
          });
        }
      } catch (error) {
        if (cancelled || controller.signal.aborted) {
          setPersonalization((current) =>
            current.loading ? { ...current, loading: false } : current,
          );
          return;
        }
        setPersonalization({
          data: null,
          error: error instanceof Error ? error.message : "Failed to load personalization",
          loading: false,
          loaded: true,
        });
        if (error instanceof ApiRequestError && error.status === 401) {
          markUnauthorized();
        }
      }
    }

    async function loadChannels() {
      if (channels.loaded || channels.loading) {
        return;
      }
      setChannels((current) => ({ ...current, loading: true, error: null }));
      try {
        const data = await abilitiesApi.loadChannels({ signal: controller.signal });
        if (!cancelled) {
          setChannels({
            data,
            error: null,
            loading: false,
            loaded: true,
          });
        }
      } catch (error) {
        if (cancelled || controller.signal.aborted) {
          setChannels((current) =>
            current.loading ? { ...current, loading: false } : current,
          );
          return;
        }
        setChannels({
          data: null,
          error: error instanceof Error ? error.message : "Failed to load channels",
          loading: false,
          loaded: true,
        });
        if (error instanceof ApiRequestError && error.status === 401) {
          markUnauthorized();
        }
      }
    }

    async function loadSkills() {
      if (skills.loaded || skills.loading) {
        return;
      }
      setSkills((current) => ({ ...current, loading: true, error: null }));
      try {
        const data = await abilitiesApi.loadSkills({ signal: controller.signal });
        if (!cancelled) {
          setSkills({
            data,
            error: null,
            loading: false,
            loaded: true,
          });
        }
      } catch (error) {
        if (cancelled || controller.signal.aborted) {
          setSkills((current) =>
            current.loading ? { ...current, loading: false } : current,
          );
          return;
        }
        setSkills({
          data: null,
          error: error instanceof Error ? error.message : "Failed to load skills",
          loading: false,
          loaded: true,
        });
        if (error instanceof ApiRequestError && error.status === 401) {
          markUnauthorized();
        }
      }
    }

    if (activeSection === "personalization") {
      void loadPersonalization();
    } else if (activeSection === "channels") {
      void loadChannels();
    } else {
      void loadSkills();
    }

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [
    activeSection,
    authRevision,
    canAccessProtectedApi,
    markUnauthorized,
    channels.loaded,
    personalization.loaded,
    skills.loaded,
  ]);

  function reloadSection(section: AbilitiesSection) {
    if (section === "personalization") {
      setPersonalization(idleState());
      return;
    }
    if (section === "channels") {
      setChannels(idleState());
      return;
    }
    setSkills(idleState());
  }

  return {
    personalization,
    channels,
    skills,
    reloadSection,
  };
}
