# Prompt And Personality

## User Story

As a LoongClaw operator, I want native prompt and personality presets so that I
can start with a consistent LoongClaw identity without manually writing a full
system prompt.

## Acceptance Criteria

- [ ] LoongClaw has a native base prompt owned by the product rather than only a
      free-form prompt string.
- [ ] Onboarding offers seven default personalities:
      `classicist`, `pragmatist`, `idealist`, `romanticist`, `hermit`,
      `cyber_radical`, and `nihilist`.
- [ ] Personality metadata is defined in one shared catalog so prompt rendering,
      onboarding selection, and CLI validation do not drift apart.
- [ ] Onboarding clearly labels experimental personalities so operators can
      distinguish sharper presets from the stable baseline set.
- [ ] Legacy personality ids from the earlier three-preset rollout continue to
      load and map onto supported personalities so existing configs remain
      readable.
- [ ] All personalities share the same safety-first operating boundaries.
- [ ] Personality selection can affect tone and action style without weakening
      security requirements.
- [ ] Runtime identity overlays are resolved separately from the native base
      prompt so workspace `IDENTITY.md` context can take precedence over legacy
      imported identity without replacing LoongClaw's product-owned baseline.
- [ ] Non-interactive onboarding supports personality selection with a stable
      CLI flag.
- [ ] Advanced users can still provide a full inline system prompt override.

## Personality Catalog Summary

| Id | Intent | Notes |
| --- | --- | --- |
| `classicist` | Formal, precise, orderly | Default-safe baseline aligned with the existing calm-engineering tone |
| `pragmatist` | Lean, decisive, outcome-first | Best when operators want direct execution energy |
| `idealist` | Principled, long-horizon, mission-driven | Emphasizes values and durable impact |
| `romanticist` | Expressive, image-rich, metaphor-aware | Adds tasteful literary texture without hiding substance |
| `hermit` | Gentle, patient, grounding | Optimized for calm emotional tone and paced guidance |
| `cyber_radical` | Bold, unconventional, high-energy | Experimental; onboarding should label it accordingly and it must stay compliant and safety-bounded |
| `nihilist` | Dry, skeptical, darkly witty | Experimental; onboarding should label it accordingly and it must suppress dark humor in sensitive contexts |

## Out of Scope

- Arbitrary end-user personality editing in the first release
- Full workspace template pack generation
- Multi-axis personality composition beyond one preset at a time
- Migration import/nativeization flows
