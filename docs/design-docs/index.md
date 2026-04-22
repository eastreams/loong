# Design Documents Index

This directory is the repository-native map for Loong's public architecture
references.

The Mintlify site under `site/` is the public builder-facing reading path. This
index exists for contributors and source readers who need the repository
markdown behind that shorter public architecture summary.
## Read This Index When

- you need the repository-native architecture references rather than the shorter
  Mintlify summary
- you are changing runtime boundaries, layering, or engineering rules
- you want the source documents that back the public builder contract

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read the public builder-facing overview first | [`../../site/build-on-loong/overview.mdx`](../../site/build-on-loong/overview.mdx) | `site/` is the public builder hub |
| edit repository-native architecture references | this index | this directory is the source-facing architecture map |
| understand the overall repository docs layering | [`../README.md`](../README.md) | it explains how repo-native docs differ from Mintlify pages |

## What Stays Here

This directory keeps the architecture references that remain part of the OSS
surface. Deeper implementation packages, comparative studies, and internal-only
design backlog artifacts are intentionally out of the public docs flow.

## Source Design Map

| Document | Read it when... |
| --- | --- |
| [Core Beliefs](core-beliefs.md) | you need the engineering principles that should survive refactors |
| [Layered Kernel Design](layered-kernel-design.md) | you need the crate and layer boundary model before changing runtime shape |
| [Runtime Entrypoint and Bootstrap Map](runtime-entrypoint-map.md) | you need the shortest source-facing map of how CLI, channel, gateway, control-plane, and daemon task turns enter the shared runtime |
| [Automation Triggering](automation-triggering.md) | you are changing schedule, webhook, event-driven, or future hook-triggered automation and need the repository-native trigger model |
| [Single-Entry Runtime Convergence](single-entry-runtime-convergence.md) | you are working on session-vs-memory ownership or converging host turn seams without breaking the current crate contract |
| [Harness Engineering](harness-engineering.md) | you are working on the agent-driven development environment itself |
| [Tool Surface Exposure](tool-surface-exposure.md) | you are changing provider-visible tool exposure, discovery, or hidden-tool progressive disclosure |

## Boundary Rules

- keep this index focused on architecture references that remain part of the
  OSS source surface
- do not turn `design-docs/` back into a catch-all archive for backlog design
  bundles, internal comparisons, or implementation packages
- put new reader-facing explainers in `site/` when they are really public docs
  pages rather than source references

## Suggested Reading Order
1. Start with [Core Beliefs](core-beliefs.md) if you need the repository's
   architectural taste and invariants.
2. Continue to [Layered Kernel Design](layered-kernel-design.md) if the change
   touches boundaries, ownership, or layering.
3. Read [Runtime Entrypoint and Bootstrap Map](runtime-entrypoint-map.md) when
   the problem is “which runtime/bootstrap surface actually owns this turn?”
4. Read [Tool Surface Exposure](tool-surface-exposure.md) when the work touches
   provider-visible tools, discovery, or hidden-tool progressive disclosure.
5. Read [Harness Engineering](harness-engineering.md) only when the work is
   really about the development environment or agent workflow itself.
