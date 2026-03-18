## Context

`alpha-test` already has a strong provider abstraction:

- provider profiles define defaults, auth env bindings, and transport family
- onboard preflight surfaces credential, transport, and model-probe checks
- doctor reuses the same transport and probe semantics for repair guidance

The current gap is narrower: several providers have official region-specific
entrypoints, but the active defaults collapse that choice into one host. When a
valid key is used against the wrong regional host, operators often see `401`,
`403`, or model-catalog probe failures that look like bad credentials even
though the real issue is endpoint-region mismatch.

## Root cause

LoongClaw currently treats these region-sensitive providers as a single static
base URL:

- `minimax` defaults to `https://api.minimaxi.com`
- `kimi` defaults to `https://api.moonshot.cn`
- `zai` defaults to `https://api.z.ai`
- `zhipu` defaults to `https://open.bigmodel.cn`

Those defaults are internally coherent, but they hide the operator decision:
"does this key belong to the global endpoint or the mainland China endpoint?"

## Non-goals

- do not split one provider into many new provider kinds
- do not change request routing or failover semantics broadly
- do not auto-probe alternate regions during runtime
- do not warn on every healthy config just because a provider has multiple
  official regions

## Chosen approach

Add lightweight region-endpoint guidance helpers in `ProviderConfig`, then reuse
them in the existing operator surfaces:

1. review and onboarding success summaries:
   - show the current region endpoint choice clearly
2. onboard and doctor auth-style model-probe failure messaging:
   - append a targeted hint only when the provider is region-sensitive and the
     probe result is classified as an authentication or authorization rejection
     (for example `401`/`403`-like failures)
3. doctor next steps:
   - add a concrete `provider.base_url` adjustment step for those providers
4. runtime auth rejection message path:
   - when a region-sensitive provider returns `401` or `403`, append the same
     region guidance so the live failure is actionable

## Why this approach

This keeps the fix aligned with the existing architecture:

- provider knowledge stays in `config/provider.rs`
- onboarding and doctor remain thin consumers of provider metadata
- runtime behavior changes only in error presentation, not request semantics

It also avoids the two bad extremes:

- no provider-kind explosion
- no opaque "try another region" prose duplicated across CLI layers

## Initial provider scope

The first pass should cover only providers with clear evidence of official
global/CN endpoint variants:

- `minimax`
- `kimi`
- `zai`
- `zhipu`

`bedrock` already has explicit region templating and existing guidance, so it
does not need the same treatment. BytePlus and Volcengine remain path-family
guidance problems in the current codebase, not endpoint-region toggle problems.
