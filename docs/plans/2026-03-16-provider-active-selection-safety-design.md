# Provider Active Selection Safety Design

Date: 2026-03-16
Status: Approved for implementation

## Goal

Fix provider-selection drift in `alpha-test` so mixed legacy `[provider]` and
profile-based `[providers.*]` configurations preserve the user's intended
runtime provider, surface the risk through config diagnostics, and stop
misleading `doctor` / `onboard` output when model discovery is only advisory.

This design addresses the class of failures where chat traffic, provider auth
checks, and runtime diagnostics silently target a different provider than the
one the user explicitly configured.

## Problem Statement

The current config/runtime flow has three coupled hazards:

1. Mixed legacy/profile configs can silently change the active provider.
2. `validate-config` does not explain that risk.
3. `doctor` and `onboard` overstate model-probe failures even when an explicit
   `model` means runtime chat does not depend on `/models`.

The drift is global, not provider-specific. OpenRouter, Volcengine Coding, and
any other provider profile can be affected because the current fallback depends
on normalized profile storage rather than explicit selection intent.

## Root Cause

### Active-provider fallback is inferred from storage order

`LoongClawConfig::normalize_provider_profiles()` currently normalizes profile
storage and then resolves the active provider using this priority:

1. explicit `active_provider`
2. inferred legacy profile id from `config.provider`
3. `providers.keys().next()`

That final fallback is effectively lexicographic because `providers` is a
`BTreeMap`. Once `providers` is non-empty and `active_provider` is absent or
invalid, the runtime may silently switch to the alphabetically first profile.

### Deserialization loses config intent

After `toml::from_str::<LoongClawConfig>()`, the code cannot tell whether the
top-level `provider` table was explicitly present or whether `provider` just
contains default OpenAI values. That makes it impossible to distinguish:

- a user who explicitly chose a legacy provider and has not yet migrated
- a profile-only config with no legacy provider intent

### Validation only checks one branch of config shape

`collect_validation_issues()` validates:

- the top-level `provider` only when `providers` is empty
- saved `providers.<id>` only when `providers` is non-empty

That means mixed configs do not receive any diagnostic explaining that the
top-level provider and saved profiles disagree or that the active provider is
being inferred implicitly.

### Doctor / onboard do not separate runtime viability from catalog viability

`doctor` and `onboard` currently treat `fetch_available_models()` failure as a
hard failure whenever credentials exist. But explicit model mode already allows
runtime chat to proceed without catalog discovery because the request path can
use the configured model directly.

## Design Principles

### Principle 1: Explicit user selection wins over container order

Runtime provider selection must never depend on `BTreeMap` ordering when the
user gave an explicit legacy or profile selection signal.

### Principle 2: Compatibility must remain readable, but not silent

Legacy `[provider]` stays supported. Mixed legacy/profile states are allowed to
load for compatibility, but risky states must be surfaced as diagnostics.

### Principle 3: Diagnostics need severity

The system needs warning-level diagnostics for dangerous-but-loadable config
states. Fatal parse/validation errors remain possible, but not every issue is a
hard stop.

### Principle 4: Runtime checks must describe what they actually prove

`/models` probing proves catalog availability. It does not always prove whether
chat will fail. Diagnostics must distinguish those two facts.

## Target Behavior

### Config loading and normalization

When a config mixes legacy `[provider]` and saved `[providers.*]`:

- if `active_provider` is explicit and valid, use it
- if `active_provider` is explicit but missing in `providers`, keep loading but
  emit a warning and recover deterministically
- if `active_provider` is absent and legacy `[provider]` was explicitly set,
  preserve that legacy runtime choice instead of falling back to the first map
  entry
- if necessary, synthesize or match a provider profile that corresponds to the
  explicit legacy provider and mark it active
- only use a stable fallback when there is genuinely no explicit selection
  intent anywhere

### Validation output

`validate-config` should emit diagnostics with severity, at minimum:

- `error` for invalid env pointer / numeric / unknown account diagnostics that
  already fail semantic validation today
- `warn` for dangerous mixed provider-selection states that remain loadable

Important warning scenarios:

- mixed legacy `[provider]` plus `[providers.*]` with no explicit
  `active_provider`
- explicit `active_provider` that does not resolve to any saved profile and
  forced recovery occurs
- active provider being reconstructed from legacy intent because profile-based
  selection is incomplete

The CLI should keep `valid=true` when there are no error diagnostics and report
warning/error counts separately. Severity exists to distinguish warnings from
hard errors and to let callers reason about policy without hiding problems.

### Doctor and onboard behavior

Provider-oriented diagnostics must clearly identify the active selection being
checked, ideally including both:

- active provider profile id
- provider kind / model strategy summary

Model-probe semantics should change:

- auto-discovery mode: model probe failure remains `FAIL`
- explicit-model mode: model probe failure becomes `WARN`, with detail that
  catalog discovery failed but chat may still work because the model is
  explicitly configured

## Implementation Strategy

### 1. Preserve raw config intent during parse

Extend `LoongClawConfig` with internal, non-serialized flags such as:

- `legacy_provider_explicit`
- `active_provider_explicit`

Populate them by first parsing the raw TOML into `toml::Value`, then
deserializing the config struct. This is the smallest change that preserves user
intent without rewriting the entire config loader.

### 2. Centralize deterministic active-provider recovery

Refactor provider normalization to:

- normalize profile ids first
- resolve active selection from explicit profile intent when possible
- otherwise recover from explicit legacy provider intent by finding or creating
  the matching profile
- avoid implicit lexicographic fallback when a stronger signal exists

### 3. Add diagnostic severity

Introduce a severity field on config diagnostics so text, JSON, and
`application/problem+json` can all distinguish warnings from errors. Existing
validation helpers stay reusable, but diagnostics become richer and future-safe.

### 4. Align doctor / onboard with provider selection and model strategy

Use the normalized active profile information in presentation and model-probe
detail strings. Explicit-model mode should produce a warning-level probe result
instead of a false hard failure.

## Non-Goals

- changing provider auth schemes or provider-specific request adapters
- redesigning runtime request execution beyond the model-probe classification
- removing legacy `[provider]` compatibility
- changing normal chat failover semantics

## Risks And Mitigations

### Risk: new severity plumbing expands validation surface

Mitigation:

- keep the initial severity model minimal: `Error` and `Warn`
- default existing diagnostics to `Error`
- add targeted tests for text, JSON, and problem-json outputs

### Risk: normalization changes can alter persisted config state

Mitigation:

- write regression tests for legacy-only, profile-only, and mixed configs
- ensure writes stay canonical and stable after reload

### Risk: doctor/onboard wording drifts from actual runtime guarantees

Mitigation:

- keep probe messages explicit about what failed: catalog lookup, not the chat
  request itself
- add tests for both explicit-model and auto-discovery paths

## Verification Plan

Add regression coverage for:

- mixed legacy/profile configs missing `active_provider`
- invalid explicit `active_provider` recovery
- warning diagnostics for provider-selection drift hazards
- `doctor` explicit-model probe downgrade to warning
- `onboard` explicit-model probe downgrade to warning
- text / JSON / problem-json validation output carrying severity

Then run focused package tests plus broader package regressions before commit and
PR.
