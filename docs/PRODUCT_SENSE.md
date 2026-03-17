# Product Sense

User-experience principles and product direction for the current LoongClaw MVP.

## Target Users

LoongClaw is not only a runtime for developers. The current MVP is aimed at:

1. **Individuals and operators** who want a private assistant they can run locally and trust.
2. **Channel operators** who want the same assistant behavior to show up in CLI, Telegram, and Feishu.
3. **Developers and extension authors** who need stable seams for providers, tools, channels, and memory.

## Product Principles

1. **First value fast** — a new user should get to a useful assistant answer quickly, not after reading implementation docs.
2. **Safe by default** — visible capabilities must still honor policy, approval, and audit boundaries.
3. **Assistant-first surfaces** — user-facing capability should feel like “my assistant can do this”, not only “the platform exposes an adapter”.
4. **Progressive disclosure** — `onboard`, `ask`, `chat`, and `doctor` carry the common path; each surface should lead with the next user action before exposing runtime detail.
5. **One runtime, many surfaces** — CLI ask, interactive chat, and future surfaces should share the same conversation, memory, tool, and provider semantics.
6. **Fail loud with a repair path** — when setup or runtime health breaks, LoongClaw must point users toward `doctor` instead of leaving them in silent failure.

## Current MVP Journey

The current product contract is:

1. Install LoongClaw through the documented bootstrap installer, which prefers
   checksum-verified GitHub Release binaries and keeps an explicit `--source`
   fallback from a local checkout.
2. Run `loongclaw onboard`.
3. Set provider credentials.
4. Get first value through a concrete one-shot command such as
   `loongclaw ask --message "Summarize this repository and suggest the best next step."`,
   then use `loongclaw chat` for follow-up interactive work.
5. If anything is broken, use `loongclaw doctor` or `loongclaw doctor --fix`.
6. Enable Telegram or Feishu only after the base CLI flow is healthy.

This keeps the first-run journey legible while preserving the existing runtime architecture.

For the current MVP, that also means first-run surfaces should feel assistant-first in their copy:
show the runnable handoff first, then keep config, memory, and runtime facts in secondary detail blocks.

## Product Specifications

See [Product Specs Index](product-specs/index.md) for detailed user-facing requirements:

- [Installation](product-specs/installation.md) — bootstrap install, release-first download, and source fallback
- [Onboarding](product-specs/onboarding.md) — first-run setup and handoff to first success
- [One-Shot Ask](product-specs/one-shot-ask.md) — non-interactive assistant fast path
- [Doctor](product-specs/doctor.md) — diagnostics and safe repair expectations
- [Browser Automation](product-specs/browser-automation.md) — bounded browser-style assistant actions
- [Channel Setup](product-specs/channel-setup.md) — configuring shipped assistant surfaces
- [Tool Surface](product-specs/tool-surface.md) — truthful runtime-visible tool advertising
- [WebChat](product-specs/webchat.md) — expectations for the browser-facing chat surface
- [Memory Profiles](product-specs/memory-profiles.md) — memory access patterns
- [Prompt And Personality](product-specs/prompt-and-personality.md) — prompt engineering constraints

## User-Facing Commands

The primary daemon surfaces are:

| Command | Purpose |
|---------|---------|
| `onboard` | Guided first-run setup, detection, and configuration |
| `ask` | One-shot assistant answer and exit |
| `chat` | Interactive CLI conversation |
| `doctor` | Health diagnostics with optional safe repair |
| `migrate` | Power-user migration path |
| `telegram-serve` / `feishu-serve` | Service channels once the base setup is healthy |

## See Also

- [Roadmap](ROADMAP.md) — stage-based milestones with user impact
- [Contributing](../CONTRIBUTING.md) — how to add channels, tools, providers
