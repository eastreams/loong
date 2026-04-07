# Personalization

## User Story

As a Loong operator, I want an optional way to teach the assistant my
working preferences so that future sessions can adapt to me without slowing
down the first-run path or mutating runtime identity authority.

## Acceptance Criteria

- [ ] Loong exposes an optional `loong personalize` command for
      operator preference capture and review.
- [ ] Personalization is separate from `loong onboard` setup and does not
      block the primary first-run path of `onboard -> ask -> chat -> doctor`.
- [ ] The flow is conversational and asks one preference at a time instead of
      presenting a large profile form.
- [ ] The first release focuses on collaboration preferences such as operator
      preferred name, response density, initiative level, standing boundaries,
      and optional timezone or locale.
- [ ] Personalization can be surfaced as a secondary next step from healthy
      operator surfaces such as onboarding success, welcome, doctor, or chat,
      but it is never the primary required action ahead of a first answer.
- [ ] Personalization results are reviewed before save and can be skipped,
      deferred, suppressed, or rerun explicitly later.
- [ ] Operators can rerun `loong personalize` to update or clear saved
      preferences, and suppression only persists an advisory do-not-suggest
      state until operators explicitly rerun and save updated preferences,
      without erasing already saved preferences.
- [ ] Persisted personalization state remains advisory and is projected through
      the session-profile lane rather than becoming a second runtime identity
      authority.
- [ ] The default persistence path for personalization does not implicitly write
      to runtime-self authority files such as `AGENTS.md`, `IDENTITY.md`,
      `USER.md`, or `SOUL.md`.
- [ ] Personalization persistence is deterministic and structured enough to
      preserve stable fields such as `preferred_name`, `response_density`,
      `initiative_level`, `standing_boundaries`, `timezone`, and a versioned
      update timestamp.
- [ ] If personalization storage fails, Loong fails loud with a repair path
      and leaves the healthy setup and current runtime identity boundary intact.

## Out of Scope

- Mandatory post-onboard identity bootstrapping
- Assistant self-naming, emoji/vibe selection, or roleplay-heavy persona setup
- Broad personal biography collection or dossier-style user profiling
- Implicit mutation of runtime-self authority files during preference capture
