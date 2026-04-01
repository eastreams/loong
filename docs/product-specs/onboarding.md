# Onboarding

## User Story

As a first-time LoongClaw user, I want a guided setup flow so that I can reach a
working assistant without editing raw config or guessing which command comes
next.

## Acceptance Criteria

- [ ] `loong onboard` is the default first-run path called out in product docs.
- [ ] Onboarding detects reusable provider, channel, or workspace settings when
      available and explains what it found before writing config.
- [ ] The happy path ends with explicit next-step guidance for:
      a concrete `loong ask --message "..."` example and `loong chat`.
- [ ] The success summary leads with a runnable `start here` handoff before the
      saved provider, prompt, memory, and channel inventory.
- [ ] The primary post-onboard handoff prefers a one-shot `ask` example before
      interactive `chat`, so first success does not require learning the REPL.
- [ ] The shared post-onboard next-step model can also surface optional browser
      preview enable, runtime install, or first-recipe guidance when that lane
      is available for the current config.
- [ ] Onboarding success may surface an optional personalization next step after
      the primary first-answer handoff, but it does not turn relationship
      bootstrapping into a required setup stage.
- [ ] Interactive onboarding explains how to exit cleanly, including an
      explicit `Esc` cancellation hint before any config write.
- [ ] Interactive fixed-choice prompts use terminal-native selection widgets
      with arrow-key navigation instead of raw numeric or exact-string entry.
- [ ] The credential-source step asks for an environment variable name,
      rejects pasted secret literals or shell assignment syntax, and never
      echoes rejected secret-like input in review or success output.
- [ ] Interactive onboarding lets the user choose a default web search provider
      and asks for a web-search credential env source immediately when that
      provider requires a key.
- [ ] Non-interactive onboarding supports `--web-search-provider` and
      `--web-search-api-key`, and explicit web-search choices are not silently
      replaced by heuristic fallbacks.
- [ ] When provider credentials are already available and catalog discovery
      succeeds, model selection offers a searchable model list while still
      allowing a manual custom model override.
- [ ] Rerunning onboarding does not silently overwrite an existing config unless
      the user explicitly opts into a destructive path such as `--force`.
- [ ] Onboarding uses the same provider, memory, and channel configuration
      surfaces that the runtime uses after setup.
- [ ] When preflight checks fail, onboarding points users to `loong doctor`
      or `loong doctor --fix` as the repair path.
- [ ] Onboarding preflight reuses the same browser companion diagnostics as
      `loong doctor`, surfacing optional managed-lane blockers before write
      without redefining runtime truth inside onboarding.
- [ ] Providers with a reviewed onboarding default model, such as MiniMax and
      DeepSeek, can complete setup with an explicit model even when model
      catalog discovery is unavailable during setup.
- [ ] `preferred_models` remains an explicit operator-configured fallback path
      rather than a hidden provider-owned runtime default.
- [ ] When model catalog discovery fails while the config still uses
      `model = auto`, onboarding gives actionable remediation: rerun onboarding
      to accept a reviewed explicit model when one exists, or set
      `provider.model` / `preferred_models` explicitly.

## Out of Scope

- Package-manager distribution strategy beyond the documented bootstrap installer;
  see [Installation](installation.md)
- Mandatory operator-personalization or relationship-building rituals during setup
- Full channel pairing or browser-based setup UIs
- Arbitrary advanced config editing during first run
