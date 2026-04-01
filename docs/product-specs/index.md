# Product Specs

This directory is the repository-native map for LoongClaw's source-facing
product contracts.

The public reader path lives under `site/`. This index exists for maintainers,
contributors, and source readers who need the markdown contracts behind those
public docs.

## Read This Index When

- you are editing the source-level contract behind a public operator workflow
- you need to see which repository markdown files define the current public
  product surface
- you want the source contract, not the Mintlify tutorial or playbook layer

## What Lives Here

Product specs describe **what** the product does from the user's perspective,
not implementation internals, backlog staging, or private productization notes.

Preview-only, future-facing, and internal productization specs are no longer
mirrored in the OSS repository by default. This directory stays focused on
shipped or near-shipped public journeys that still need a repository-native
source contract.

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read the public operator-facing docs first | [`../../site/use-loongclaw/overview.mdx`](../../site/use-loongclaw/overview.mdx) | `site/` is the main reader-facing docs surface |
| read first-run docs like a public reader | [`../../site/get-started/overview.mdx`](../../site/get-started/overview.mdx) | tutorials and onboarding flows belong there |
| edit the source-level product contract in the repository | this index | this directory is the source-facing contract map |
| understand the broader repository docs split | [`../README.md`](../README.md) | it explains the repo-native docs layering |

## Source Specs By Operating Area

| Area | Source specs | Read them when... |
| --- | --- | --- |
| first-run and local success path | [Installation](installation.md), [Onboarding](onboarding.md), [One-Shot Ask](one-shot-ask.md), [Doctor](doctor.md), [Shell Completion](shell-completion.md) | you are editing the base setup and recovery contract |
| shipped runtime surfaces and operator controls | [Browser Automation](browser-automation.md), [Channel Setup](channel-setup.md), [Tool Surface](tool-surface.md) | you are editing surface-specific setup, controls, or field-level behavior |
| runtime behavior and continuity | [Prompt And Personality](prompt-and-personality.md), [Memory Profiles](memory-profiles.md) | you are editing day-to-day operator behavior, tone, or continuity rules |

## Public Contract Notes

- `Installation`, `Onboarding`, `One-Shot Ask`, `Doctor`, `Browser Automation`,
  `Tool Surface`, and `Channel Setup` define the shipped first-run and support
  journey for the current MVP.
- `Prompt And Personality`, `Memory Profiles`, and `Shell Completion` remain
  public because they affect the current operator-facing setup and runtime
  experience directly.
- Future browser companion, Web UI, task UX, discovery UX, retrieval UX, and
  control-plane productization specs stay out of this repository index until
  they are ready to become public contracts.

## Do Not Put Here By Default

- new walkthrough-style onboarding, recipes, or playbooks that belong in
  `site/`
- preview-only productization packages, backlog specs, or longer-horizon notes
- duplicate mirrors of Mintlify navigation pages
- internal planning bundles that should live outside the OSS docs flow
