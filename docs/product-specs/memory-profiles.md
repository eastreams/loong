# Memory Profiles

## User Story

As a Loong operator, I want selectable memory profiles so that I can choose
how continuity is preserved without manually wiring different memory systems.

## Acceptance Criteria

- [ ] Loong exposes memory behavior through a user-facing `memory.profile`
      surface.
- [ ] The first release supports `window_only`, `window_plus_summary`, and
      `profile_plus_window`.
- [ ] Existing SQLite-based configs continue to work without migration.
- [ ] `window_plus_summary` injects condensed earlier session context before the
      recent sliding window.
- [ ] Hydrated advisory memory entries carry deterministic provenance metadata
      that explains source kind, scope, recall mode, trust level, authority,
      derived kind, and record status.
- [ ] `profile_plus_window` can inject a durable `profile_note` block for
      preferences, tuning, or advisory imported context.
- [ ] `profile_plus_window` remains the durable advisory lane that future recall
      may enrich without becoming a second identity authority.
- [ ] When compaction runs with a configured safe workspace root, Loong can
      export advisory durable recall into `memory/YYYY-MM-DD.md` before
      compacting context.
- [ ] When a configured safe workspace root exposes durable memory files,
      Loong can bootstrap advisory durable recall from `MEMORY.md`,
      `memory/MEMORY.md`, and recent daily logs without overriding runtime
      identity.
- [ ] Canonical session-history recall is exposed through a separate
      `session_search` surface instead of overloading durable workspace-memory
      search.
- [ ] Workspace durable recall honors explicit record status metadata so
      superseded, tombstoned, or archived files are excluded from prompt
      assembly and operator inspection.
- [ ] Session-local derived overview artifacts remain advisory and do not
      replace runtime-self guidance, resolved runtime identity, or the session
      profile.
- [ ] Legacy imported identity can still be recovered from `profile_note`, but
      it is resolved into a separate runtime identity lane rather than being
      projected back into the session profile block.
- [ ] Non-interactive onboarding supports selecting a memory profile.

## Out of Scope

- Vector retrieval or semantic search
- Multi-backend storage selection in onboarding
- Automatic LLM-generated long-term summaries
- Full migration import tooling
