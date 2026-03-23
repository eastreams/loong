# Memory Profiles

## User Story

As a LoongClaw operator, I want selectable memory profiles so that I can choose
how continuity is preserved without manually wiring different memory systems.

## Acceptance Criteria

- [ ] LoongClaw exposes memory behavior through a user-facing `memory.profile`
      surface.
- [ ] The first release supports `window_only`, `window_plus_summary`, and
      `profile_plus_window`.
- [ ] Existing SQLite-based configs continue to work without migration.
- [ ] `window_plus_summary` injects condensed earlier session context before the
      recent sliding window.
- [ ] `profile_plus_window` can inject a durable `profile_note` block for
      preferences, tuning, or advisory imported context.
- [ ] `profile_plus_window` remains the durable advisory lane that future recall
      may enrich without becoming a second identity authority.
- [ ] When compaction runs with a configured safe workspace root, LoongClaw can
      export advisory durable recall into `memory/YYYY-MM-DD.md` before
      compacting context.
- [ ] Legacy imported identity can still be recovered from `profile_note`, but
      it is resolved into a separate runtime identity lane rather than being
      projected back into the session profile block.
- [ ] Non-interactive onboarding supports selecting a memory profile.

## Out of Scope

- Vector retrieval or semantic search
- Multi-backend storage selection in onboarding
- Automatic LLM-generated long-term summaries
- Full migration import tooling
