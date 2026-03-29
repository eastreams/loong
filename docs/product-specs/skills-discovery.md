# Skills Discovery

## User Story

As a LoongClaw operator, I want a discovery-first managed skills surface so
that I can find the right skill, understand whether it is usable, and get a
clear first-use path without already knowing the exact `skill_id`.

## Acceptance Criteria

- [ ] LoongClaw exposes search and recommendation over the existing managed,
      user, and project skill inventory.
- [ ] Search results surface operator-relevant metadata including scope,
      summary, eligibility, model visibility, invocation policy, and shadowing
      status.
- [ ] The first slice explains why a candidate is blocked, shadowed, invisible,
      or manual-only instead of silently omitting that information.
- [ ] Install and inspect flows return concrete first-use guidance after the
      operator selects a skill.
- [ ] The discovery flow reuses the current managed external-skills runtime
      rather than introducing per-skill dynamic provider tool registration.
- [ ] Product docs clearly distinguish shipped discovery-first flows from any
      future remote registry, marketplace ranking, or auto-install behavior.

## Current Baseline

The current runtime already ships:

- `external_skills.fetch`
- `external_skills.install`
- `external_skills.list`
- `external_skills.inspect`
- `external_skills.invoke`
- `external_skills.remove`
- `external_skills.policy`
- operator CLI support for list, info, fetch, install, bundled install, remove,
  and browser-preview enablement

The current gap is not managed lifecycle. It is discovery-first operator UX.

## Out of Scope

- blind remote auto-install
- arbitrary script execution during install
- per-skill dynamic function-tool registration
- marketplace reputation systems and social ranking
- Web UI skill marketplace surfaces
