# Tool Surface

## User Story

As a LoongClaw user, I want the assistant to advertise only the tools that are
actually available in my current runtime, so that the product feels truthful
and I do not get routed into disabled or still-planned capabilities.

## Acceptance Criteria

- [ ] Capability snapshots, provider tool schemas, and conversation tool views
      are derived from the same runtime-visible tool policy.
- [ ] Tools that are compiled out, disabled by config, or unavailable on the
      current surface are not advertised as callable.
- [ ] Tools that remain visible in order to unlock a capability, such as
      `external_skills.policy`, are explicitly treated as enablement surfaces,
      while the corresponding lifecycle or invoke tools remain hidden until that
      capability is enabled.
- [ ] User-facing docs and product specs describe the shipped tool surface with
      the same canonical tool names that the runtime executes.

## Out of Scope

- Tool ranking or prompt tuning strategy
- Per-provider formatting differences that do not change the visible tool set
- Long-term expansion of the tool catalog beyond the current MVP
