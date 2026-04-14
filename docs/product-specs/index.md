# Product Specs

This directory is the repository-native map for Loong's source-facing product
contracts.

The public reader path lives under `site/`. This index exists for maintainers,
contributors, and source readers who need the repository-native contracts that
still live outside the Mintlify docs surface.

## Read This Index When

- you need the source contract behind a shipped or near-shipped operator flow
- you are deciding whether a doc should stay repository-native or move to `site/`
- you want the maintainer-facing contract instead of the public tutorial layer

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read the public operator-facing docs first | [`../../site/use-loong/overview.mdx`](../../site/use-loong/overview.mdx) | `site/` is the main reader-facing docs surface |
| read the public field-level config reference | [`../../site/use-loong/configuration-reference.mdx`](../../site/use-loong/configuration-reference.mdx) | the config reference now belongs to the main docs surface |
| read the current repository-native product contract that still lives here | [Background Tasks](background-tasks.md) | this directory now keeps only the remaining source-facing contracts that are not part of the main public docs flow |
| understand the broader repository docs split | [`../README.md`](../README.md) | it explains the repo-native docs layering |

## Current Repository-Native Specs

| Area | Source specs | Read them when... |
| --- | --- | --- |
| async delegated work | [Background Tasks](background-tasks.md) | you are editing the operator contract for task-shaped background work that remains source-facing |

## Public Contract Notes

- the public field-level configuration reference now lives in
  [`../../site/use-loong/configuration-reference.mdx`](../../site/use-loong/configuration-reference.mdx)
  so the main docs surface owns the config walkthrough and reference path
- this directory should stay small and only hold source-facing product contracts
  that are still better maintained in the repository than in Mintlify
- new walkthroughs, recipes, and public operator docs should default to `site/`

## Do Not Put Here By Default

- new landing-page or tutorial content that belongs under `site/`
- duplicate mirrors of Mintlify navigation pages
- internal planning bundles or backlog-heavy design notes
- public config reference copies that would drift from the main docs surface
