# Provider Onboarding Default Model Design

## Problem

The current alpha-test fix correctly improves onboarding exit UX, but the
provider-model part is still shaped incorrectly.

The branch currently seeds provider-owned `preferred_models` and lets runtime,
onboarding, and doctor treat those hidden defaults as an acceptable fallback
when model catalog discovery fails.

That creates three avoidable problems:

- provider model names must now be maintained as hidden runtime defaults across
  many providers
- first-run behavior depends on invisible fallback state instead of explicit
  configuration the user can inspect
- onboarding, doctor, and runtime blur together: a catalog failure can look
  "fixed" even though the actual configured model is still `auto`

The MiniMax `401` report exposed this issue first, but the design problem is
broader than MiniMax.

## Goal

Keep the validated onboarding exit improvements, but replace hidden provider
runtime fallbacks with a simpler and more explicit model-selection rule:

1. onboarding may suggest or apply a provider-specific recommended model
2. runtime should only use:
   - an explicit configured model
   - a successfully fetched or cached model catalog
   - user-configured `preferred_models`
3. doctor/onboarding messaging should only promise fallback behavior when it was
   explicitly configured by the operator

## Non-Goals

- building a full provider model registry with live metadata fetch
- auditing and seeding recommended onboarding models for every provider in this
  slice
- removing user-configurable `preferred_models`
- redesigning provider migration or general config import

## Constraints

- keep the fix small and local to the active branch/PR
- preserve the already-correct `Esc` onboarding cancellation behavior
- avoid introducing new hidden defaults in runtime
- keep existing user-configured `preferred_models` behavior intact

## Approach Options

### Option A: Keep hidden provider fallback lists and expand them

Pros:

- minimal code churn from the current branch
- more providers could keep `model = auto` when discovery fails

Cons:

- turns provider model maintenance into an open-ended hidden-default catalog
- runtime behavior drifts from what the user explicitly configured
- every new provider needs curated hidden model names or inconsistent behavior

### Option B: Remove hidden runtime defaults and use onboarding-only explicit defaults

Pros:

- matches the shape used by OpenClaw: provider defaults are explicit in setup,
  not magical runtime guesses
- fixes MiniMax first-run success without introducing a generalized hidden
  fallback mechanism
- keeps runtime semantics simple and auditable

Cons:

- providers without a recommended onboarding model still need catalog discovery
  or manual `--model`
- requires a small onboarding-specific provider metadata path

### Option C: Remove all fallback behavior including user-configured
`preferred_models`

Pros:

- simplest runtime logic

Cons:

- removes an existing explicit operator control with real value
- broader behavior change than the current issue requires

## Decision

Choose Option B.

The right fix is not "never maintain provider defaults". A better approach is to
keep those defaults in the onboarding/configuration layer where they are
explicit and user-visible, while removing hidden runtime fallback defaults.

This keeps the MiniMax repair path, aligns better with OpenClaw's model setup
shape, and avoids turning `preferred_models` into an internal provider catalog.

## Design

### 1. Add onboarding-only provider default model metadata

Introduce a provider metadata method for recommended onboarding defaults, scoped
to first-run setup.

Initial scope:

- MiniMax: recommended onboarding model `MiniMax-M2.5`

Important boundary:

- this metadata is not used as a runtime fallback
- it is only consulted when onboarding is choosing a model and the current value
  is still `auto`

### 2. Change onboarding model selection behavior

When onboarding resolves the model:

- if the operator passed `--model`, use it
- else if the current provider model is explicit, keep it
- else if the provider has a recommended onboarding model, prefill and accept
  that explicit model
- else keep `auto`

This should apply to both interactive and non-interactive onboarding so
`loongclaw onboard --provider minimax` produces a usable explicit model without
requiring a live model-list probe.

### 3. Remove hidden provider default fallbacks

Delete the provider-level built-in `default_preferred_models()` usage for
runtime fallback seeding.

Behavior after change:

- `preferred_models` remains supported when the user configures it
- `configured_auto_model_candidates()` returns only explicit operator-configured
  values
- runtime no longer invents provider fallback candidates from provider kind

### 4. Tighten doctor and onboarding messaging

If model probe fails:

- explicit model: warn, but say chat may still work because the model is
  explicitly configured
- user-configured `preferred_models`: warn, but say runtime will try configured
  fallbacks
- otherwise: fail

This keeps the message truthful and operator-auditable.

### 5. Documentation

Update the onboarding spec and README copy to describe explicit provider
defaults rather than hidden preferred fallbacks.

## Testing Strategy

Add or adjust tests to prove:

- MiniMax onboarding default model is explicit and user-visible
- non-interactive onboarding with MiniMax defaults to the explicit recommended
  model instead of hidden runtime fallback state
- `fresh_for_kind(ProviderKind::Minimax)` no longer seeds hidden
  `preferred_models`
- runtime fallback still works for user-configured `preferred_models`
- doctor/onboarding only report fallback continuation for explicitly configured
  preferred models

## Why This Is Simpler

This design removes hidden behavior instead of adding more heuristics.

It keeps provider-specific knowledge in the one place where users expect it
(setup defaults) and keeps runtime behavior tied to explicit config or real
catalog data. That is a smaller and more explainable system than a growing
runtime list of provider-owned fallback model IDs.
