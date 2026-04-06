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
- [ ] Recall surfaces distinguish canonical session history tools such as
      `session_search` from workspace durable-memory tools such as
      `memory_search` and `memory_get`.
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

## Current Tool Surface Notes

- `shell.exec` remains the existing shell-execution surface.
- `bash.exec` is a shipped experimental parallel tool. It may be advertised only when the runtime can actually execute it, but it does not replace `shell.exec`.
- User-facing docs must describe `bash.exec` with its canonical tool name and must not imply that shell governance has already converged on a single execution surface.
