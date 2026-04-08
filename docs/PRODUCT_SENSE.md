# Product Sense

This file is the repository-native product-direction reference for the current
Loong MVP.

The reader-facing summary for this material lives in
[`../site/reference/roadmap-and-product.mdx`](../site/reference/roadmap-and-product.mdx).
This file remains the repository-native product reference for maintainers and
source readers.

## What This File Covers

- user-experience principles behind the current MVP
- the source-level rationale behind the current first-run journey
- the product-direction boundary behind public operator docs and source specs

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read the public summary first | [`../site/reference/roadmap-and-product.mdx`](../site/reference/roadmap-and-product.mdx) | that is the reader-facing product and roadmap summary |
| review the source-level product direction in the repository | this file | this file holds the repository-native product rationale |
| edit the user-facing source contract behind the docs site | [`product-specs/index.md`](product-specs/index.md) | product specs carry the field-level source contracts |
| understand the broader repository docs layering | [`README.md`](README.md) | it explains how repo-native docs differ from Mintlify pages |

## Read This File When

- you are editing user-facing product direction at the source level
- you need the product principles behind the public docs copy
- you need the repository-native rationale behind the current MVP journey

## What Stays Here

This file is intentionally narrower than a roadmap plus tutorial plus backlog
bundle. It should explain the current product direction and the principles that
shape public operator behavior, without turning into a second onboarding flow
or a dumping ground for future product notes.

## Target Users

Loong is not only a runtime for developers. The current MVP is aimed at:

1. **Individuals and operators** who want a private assistant they can run locally and trust.
2. **Channel and workflow operators** who want the same assistant behavior to extend from the local CLI into gateway-backed service channels and config-backed outbound delivery surfaces.
3. **Developers and extension authors** who need stable seams for providers, tools, channels, and memory.

## Product Principles

1. **First value fast** — a new user should get to a useful assistant answer quickly, not after reading implementation docs.
2. **Safe by default** — visible capabilities must still honor policy, approval, and audit boundaries.
3. **Assistant-first surfaces** — user-facing capability should feel like “my assistant can do this”, not only “the platform exposes an adapter”.
4. **Progressive disclosure** — `onboard`, `ask`, `chat`, and `doctor` carry the common path; each surface should lead with the next user action before exposing runtime detail.
5. **One runtime, one local control plane, many surfaces** — CLI ask, interactive chat, and future HTTP or browser surfaces should share the same conversation, memory, tool, provider, and session semantics.
6. **Fail loud with a repair path** — when setup or runtime health breaks, Loong must point users toward `doctor` instead of leaving them in silent failure.

## Current MVP Journey

The current product contract is:

1. Install Loong through the documented bootstrap installer, which prefers
   checksum-verified GitHub Release binaries and keeps an explicit `--source`
   fallback from a local checkout.
2. Run `loong onboard`.
3. Set provider credentials.
4. Get first value through a concrete one-shot command such as
   `loong ask --message "Summarize this repository and suggest the best next step."`,
   then use `loong chat` for follow-up interactive work.
5. If anything is broken, use `loong doctor` or `loong doctor --fix`.
6. Enable gateway or channel surfaces only after the base CLI flow is healthy.

This keeps the first-run journey legible while preserving the existing runtime architecture.

For the current MVP, that also means first-run surfaces should feel assistant-first in their copy:
show the runnable handoff first, then keep config, memory, and runtime facts in secondary detail blocks.

Future browser-based and richer product surfaces are still being designed, but
those drafts no longer live in the public repository until the user-facing
contract is ready for broad external readers.

## Source Contract Map

If you need the detailed source contract behind this product reference, start
with [Product Specs Index](product-specs/index.md).

| If you need source details about... | Start here |
| --- | --- |
| first-run success and repair | [Installation](product-specs/installation.md), [Onboarding](product-specs/onboarding.md), [One-Shot Ask](product-specs/one-shot-ask.md), [Doctor](product-specs/doctor.md) |
| shipped runtime surfaces | [Channel Setup](product-specs/channel-setup.md), [Tool Surface](product-specs/tool-surface.md), [Browser Automation](product-specs/browser-automation.md) |
| continuity and day-to-day runtime behavior | [Memory Profiles](product-specs/memory-profiles.md), [Prompt And Personality](product-specs/prompt-and-personality.md), [Shell Completion](product-specs/shell-completion.md) |

## Public Surface Shape

The current product surface is intentionally legible:

- first-run path: `onboard`, `ask`, `chat`, `doctor`
- operator runtime controls: `audit`, `migrate`, and related support commands
- longer-lived service ownership: `gateway run`, `gateway status`, `gateway stop`
- shipped service-channel runtimes: `telegram-serve`, `feishu-serve`,
  `matrix-serve`, `wecom-serve`, `multi-channel-serve`
- outbound delivery: channel `*-send` commands for the shipped outbound surface
  inventory

## Do Not Put Here By Default

- detailed onboarding, recipe, or playbook material that belongs in `site/`
- source-level setup contracts that belong in `product-specs/`
- internal planning bundles, backlog exploration, or private product studies
- speculative product directions that are not ready to become public contract
  material

## See Also

- [Roadmap](ROADMAP.md) — stage-based milestones with user impact
- [Contributing](../CONTRIBUTING.md) — how to add channels, tools, providers
