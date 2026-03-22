# Prompt And Personality

## User Story

As a LoongClaw operator, I want native prompt and personality presets so that I
can start with a consistent LoongClaw identity without manually writing a full
system prompt.

## Acceptance Criteria

- [ ] LoongClaw has a native base prompt owned by the product rather than only a
      free-form prompt string.
- [ ] Onboarding offers three default personalities:
      `calm_engineering`, `friendly_collab`, and `autonomous_executor`.
- [ ] All personalities share the same safety-first operating boundaries.
- [ ] Personality selection can affect tone and action style without weakening
      security requirements.
- [ ] Runtime identity overlays are resolved separately from the native base
      prompt so workspace `IDENTITY.md` context can take precedence over legacy
      imported identity without replacing LoongClaw's product-owned baseline.
- [ ] Non-interactive onboarding supports personality selection with a stable
      CLI flag.
- [ ] Advanced users can still provide a full inline system prompt override.

## Out of Scope

- Arbitrary end-user personality editing in the first release
- Full workspace template pack generation
- Migration import/nativeization flows
