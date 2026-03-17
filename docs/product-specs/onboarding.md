# Onboarding

## User Story

As a first-time LoongClaw user, I want a guided setup flow so that I can reach a
working assistant without editing raw config or guessing which command comes
next.

## Acceptance Criteria

- [ ] `loongclaw onboard` is the default first-run path called out in product docs.
- [ ] Onboarding detects reusable provider, channel, or workspace settings when
      available and explains what it found before writing config.
- [ ] The happy path ends with explicit next-step guidance for:
      a concrete `loongclaw ask --message "..."` example and `loongclaw chat`.
- [ ] The success summary leads with a runnable `start here` handoff before the
      saved provider, prompt, memory, and channel inventory.
- [ ] The primary post-onboard handoff prefers a one-shot `ask` example before
      interactive `chat`, so first success does not require learning the REPL.
- [ ] The shared post-onboard next-step model can also surface optional browser
      preview enable, runtime install, or first-recipe guidance when that lane
      is available for the current config.
- [ ] Rerunning onboarding does not silently overwrite an existing config unless
      the user explicitly opts into a destructive path such as `--force`.
- [ ] Onboarding uses the same provider, memory, and channel configuration
      surfaces that the runtime uses after setup.
- [ ] When preflight checks fail, onboarding points users to `loongclaw doctor`
      or `loongclaw doctor --fix` as the repair path.
- [ ] Onboarding preflight reuses the same browser companion diagnostics as
      `loongclaw doctor`, surfacing optional managed-lane blockers before write
      without redefining runtime truth inside onboarding.

## Out of Scope

- Package-manager distribution strategy beyond the documented bootstrap installer;
  see [Installation](installation.md)
- Full channel pairing or browser-based setup UIs
- Arbitrary advanced config editing during first run
