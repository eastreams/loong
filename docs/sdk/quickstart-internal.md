# Internal Integration Quickstart

Use this guide when you are adding or evolving a repository-owned surface in
Loong.

## Start Here

Read:

- [Internal Integration SDK Contract](../design-docs/internal-integration-sdk-contract.md)
- [SDK Strategy](../design-docs/sdk-strategy.md)

## Family Starting Points

| Family | Primary code seam | Primary contract doc |
|--------|-------------------|----------------------|
| Channels | `crates/app/src/channel/sdk.rs`, `crates/app/src/channel/registry.rs` | [Internal Integration SDK Contract](../design-docs/internal-integration-sdk-contract.md) |
| Tools | `crates/app/src/tools/catalog.rs`, `crates/app/src/tools/mod.rs` | [SDK Strategy](../design-docs/sdk-strategy.md) |
| Providers | `crates/app/src/config/provider.rs`, `crates/app/src/provider/contracts.rs`, `crates/app/src/provider/mod.rs` | [Provider SDK Convergence Plan](../plans/2026-03-29-provider-sdk-convergence-implementation-plan.md) |
| Memory systems | `crates/app/src/memory/system_registry.rs` | [Internal Integration SDK Contract](../design-docs/internal-integration-sdk-contract.md) |

## Maintainer Flow

1. Start from the family-owned seam instead of scattered call sites.
2. Declare canonical identity once.
3. Attach validation and support facts beside the seam.
4. Version descriptor documents once they start feeding cross-surface JSON or SDK-facing read models.
5. Implement runtime behavior after the descriptor or contract is clear.
6. Project through shared surfaces such as config, doctor, status, catalog, or
   docs.
7. Add family-specific conformance tests.
8. Update docs using the same canonical vocabulary.

## Practical Checklist

- [ ] I started from the family-owned seam.
- [ ] The surface has one canonical identity.
- [ ] Validation and support facts live beside the descriptor or contract.
- [ ] Cross-surface descriptor documents are versioned before they become shared JSON contracts.
- [ ] Runtime behavior consumes shared metadata instead of redefining it.
- [ ] Operator surfaces derive from the same family metadata where practical.
- [ ] Conformance or regression tests were added or extended.

## See Also

- [SDK Docs Index](index.md)
- [Provider SDK Convergence Plan](../plans/2026-03-29-provider-sdk-convergence-implementation-plan.md)
