# Design Documents Index

This directory is the repository-native map for LoongClaw's public architecture
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
| read the public builder-facing overview first | [`../../site/build-on-loongclaw/overview.mdx`](../../site/build-on-loongclaw/overview.mdx) | `site/` is the public builder hub |
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
| [Harness Engineering](harness-engineering.md) | you are working on the agent-driven development environment itself |

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
3. Read [Harness Engineering](harness-engineering.md) only when the work is
   really about the development environment or agent workflow itself.
