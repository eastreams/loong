# SDK Docs

Practical entrypoint for Loong SDK-related work.

Loong's SDK is not one helper crate.
It is a set of contracts and workflows for capability:

- authoring
- integration
- acquisition
- validation
- promotion

## Who This Is For

### Internal maintainers

Start with:

- [Internal Integration Quickstart](quickstart-internal.md)
- [Internal Integration SDK Contract](../design-docs/internal-integration-sdk-contract.md)
- [Provider SDK Convergence Plan](https://github.com/eastreams/knowledge-base/tree/main/loong/implementation-plans)

### External authors

Start with:

- [External Authoring Quickstart](quickstart-external.md)
- [External Authoring Contract](../design-docs/external-authoring-contract.md)
- [SDK Validator Contract](../design-docs/sdk-validator-contract.md)
- [SDK Stability Policy](../design-docs/sdk-stability-policy.md)

### Promotion and capability-evolution work

Start with:

- [Capability Promotion Contract](../design-docs/capability-promotion-contract.md)
- [SDK Strategy Implementation Roadmap](https://github.com/eastreams/knowledge-base/tree/main/loong/implementation-plans)

## Document Map

| Document | Use it for |
|----------|------------|
| [Internal Integration Quickstart](quickstart-internal.md) | Adding or refactoring repository-owned surfaces |
| [Provider SDK Convergence Plan](https://github.com/eastreams/knowledge-base/tree/main/loong/implementation-plans) | Converging the provider family into a clearer maintainer-facing seam |
| [External Authoring Quickstart](quickstart-external.md) | Understanding what external authors should build today |
| `loong plugins init` + `loong plugins invoke-extension` | Scaffold and smoke-test the shortest native extension authoring path |
| [Compatibility Matrix](compatibility-matrix.md) | Stability and maturity boundaries across SDK surfaces |
| [SDK Strategy](../design-docs/sdk-strategy.md) | Overall architecture framing |
| [SDK Stability Policy](../design-docs/sdk-stability-policy.md) | What is stable, additive, experimental, or internal |
| [SDK Validator Contract](../design-docs/sdk-validator-contract.md) | What Loong validates for capability artifacts |
| [External Authoring Contract](../design-docs/external-authoring-contract.md) | Public package and artifact authoring contract |
| [Capability Promotion Contract](../design-docs/capability-promotion-contract.md) | Governed codification path from runtime evidence to durable assets |

## Current Reading Of The Repository

Loong is not best understood as a `discovery-first` product anymore.

The current direction is:

- `discovery-first` as a lower-layer substrate
- `product mode` as the capability-acquisition surface
- autonomy-policy as the internal decision kernel
- governed promotion as the path from runtime evidence to durable capability
  assets
