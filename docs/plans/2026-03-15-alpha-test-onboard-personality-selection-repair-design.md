# Alpha-Test Onboard Personality Selection Repair Design

## Context

`alpha-test` currently exposes three conflicting product surfaces for onboarding:

- English docs describe personality selection and memory-profile selection as
  first-class onboarding capabilities.
- Product specs still define those capabilities as acceptance criteria.
- The current onboarding implementation only exposes provider, model,
  credential-env, and `system_prompt` editing.

This drift is not only a docs problem. The current onboarding flow also writes
`cli.system_prompt` without disabling the native prompt pack, which means the
visible "system prompt" customization path can diverge from the runtime prompt
that the provider layer actually uses.

## Problem Statement

The current branch regressed from an earlier implementation that supported:

- interactive personality selection
- non-interactive `--personality`
- interactive memory profile selection
- non-interactive `--memory-profile`
- explicit inline prompt override semantics that disabled the native prompt pack

That behavior disappeared during the 2026-03-14 onboarding/import unification
refactor. The regression was not caught because new onboarding tests focused on
review and shortcut UX, not on prompt-pack behavior or onboarding surface
parity.

## Goals

- Restore interactive personality selection in `loongclaw onboard`.
- Restore non-interactive `--personality`.
- Restore interactive memory profile selection.
- Restore non-interactive `--memory-profile`.
- Make inline system prompt override semantics truthful again by disabling the
  native prompt pack when onboarding captures an explicit inline prompt.
- Keep the unified onboarding/import flow and current review UX intact.
- Align English and Chinese README wording with the actual repaired behavior.
- Add regression tests that fail before the fix and pass after it.

## Non-Goals

- Reworking the import architecture beyond the minimum needed to preserve CLI
  prompt semantics.
- Adding arbitrary user-defined personalities.
- Changing runtime prompt composition outside of onboarding/import regression
  repair.
- Broad wording cleanup outside of the affected onboarding/docs surfaces.

## Constraints

- The fix must preserve the current `alpha-test` guided onboarding structure
  unless a behavior change is necessary for correctness.
- `cli.prompt_pack_id`, `cli.personality`, `cli.system_prompt_addendum`, and
  `memory.profile` must remain compatible with existing config loading.
- Migration/import logic must not silently overwrite user-owned CLI identity
  metadata in ways that conflict with the restored onboarding semantics.

## Options Considered

### Option 1: Docs-only rollback

Remove personality/memory claims from docs and leave onboarding as-is.

Rejected:

- The runtime already has native prompt-pack metadata and memory profiles.
- The current `system_prompt` onboarding path is likely misleading at runtime.
- This would normalize a regression instead of fixing it.

### Option 2: Restore the old onboarding logic wholesale

Revert large parts of the pre-unification onboarding implementation.

Rejected:

- It would discard useful review, shortcut, and starting-point UX added by the
  later refactor.
- It creates a larger merge surface than needed.

### Option 3: Repair the unified flow in place

Keep the current onboarding/import architecture, but reintroduce explicit
personality and memory-profile handling and restore correct inline prompt
override semantics.

Recommended:

- Smallest change that fixes the regression at the product seam.
- Preserves the newer onboarding/import review model.
- Lets tests pin the intended behavior in the current architecture.

## Proposed Design

### Onboarding Surface

Extend `OnboardCommandOptions` and the clap `Onboard` command with:

- `--personality`
- `--memory-profile`

Restore guided onboarding steps so the detailed flow becomes:

1. provider
2. model
3. credential source
4. personality
5. prompt addendum or inline override
6. memory profile
7. review

The exact wording should follow the current polished onboarding copy style.

### Prompt Semantics

Split onboarding CLI behavior into two explicit paths:

- Native prompt-pack path:
  - keep `prompt_pack_id`
  - set `personality`
  - set or clear `system_prompt_addendum`
  - call `refresh_native_system_prompt()`
- Inline override path:
  - clear `prompt_pack_id`
  - clear `personality`
  - clear `system_prompt_addendum`
  - store the explicit inline `system_prompt`

Interactive onboarding should default to native prompt-pack selection. Advanced
operators can still pass `--system-prompt` to switch into full inline override
mode.

### Memory Profile Semantics

Restore onboarding support for:

- `window_only`
- `window_plus_summary`
- `profile_plus_window`

Interactive onboarding should surface the current/default memory profile.
Non-interactive onboarding should accept and validate `--memory-profile`.

### Import And CLI Domain Consistency

The current migration discovery/planner code still treats the CLI domain mostly
as `system_prompt + exit_commands`. That is too narrow for the repaired
onboarding model.

The minimal repair is:

- include prompt-pack metadata and addendum in CLI-diff detection
- preserve inline override vs native-pack semantics when supplementing CLI
  config
- avoid treating a personality-only or addendum-only change as "default CLI"

This keeps import/review output consistent with the runtime and avoids future
drift.

### Docs Alignment

- English README: keep the personality and memory-profile sections, but make
  the wording truthful to the restored behavior.
- Chinese README: add matching onboarding personality/memory-profile content so
  both READMEs describe the same surface.
- Product specs remain acceptance criteria docs; do not mark them complete in
  prose, but ensure README wording does not outrun the repaired implementation.

## Testing Strategy

Add failing tests first for:

- clap parsing of `--personality` and `--memory-profile`
- non-interactive personality/memory-profile application
- interactive detailed flow transcript showing personality and memory profile
  steps
- inline `--system-prompt` path clearing prompt-pack metadata
- current unified-flow import/discovery CLI-domain comparisons correctly
  respecting prompt-pack metadata

Validation after implementation:

- targeted daemon/app tests for the new regression coverage
- broader `cargo test` scope for daemon/app if build queue permits
- formatting and clippy before PR

## Risks

- Reintroducing the old behavior too literally could conflict with the current
  onboarding review model.
- Import/migration logic may have hidden assumptions that only compare
  `system_prompt`.
- README updates could accidentally claim more than the restored runtime
  actually guarantees.

## Success Criteria

The fix is complete when:

- `loongclaw onboard --help` exposes `--personality` and `--memory-profile`
- interactive onboarding visibly offers personality and memory-profile choices
- inline prompt override semantics are honored by runtime prompt resolution
- docs in both languages match the shipped behavior
- regression tests fail before the change and pass after it
