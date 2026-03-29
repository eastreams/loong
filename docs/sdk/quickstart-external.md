# External Authoring Quickstart

Use this guide when you want the shortest practical summary of what LoongClaw
expects external authors to build.

## Read This First

- [External Authoring Contract](../design-docs/external-authoring-contract.md)
- [SDK Validator Contract](../design-docs/sdk-validator-contract.md)
- [SDK Stability Policy](../design-docs/sdk-stability-policy.md)

## Public Stance

LoongClaw's public SDK is contract-first and artifact-first.

Do not assume the stable public surface is:

- internal `crates/app` helper layout
- internal registries
- repository-only helper functions

Instead, the public surface is moving toward:

- package metadata
- package layout
- setup semantics
- validation
- controlled runtime lanes
- install, inspect, and audit behavior

## Which Family Fits?

### Managed skill

Best fit when the capability is reusable procedural guidance and should stay
installable and inspectable.

### Governed plugin package

Best fit when the capability needs a runtime lane, setup metadata, and explicit
ownership intent.

### Workflow or flow asset

Best fit when the behavior is more structured than prompt guidance and belongs
closer to reusable orchestration.

## Validation

Use [SDK Validator Contract](../design-docs/sdk-validator-contract.md) when you
need to understand the line between:

- artifact-shape validation
- doctor and setup readiness
- install or activation failures
- runtime policy denials
