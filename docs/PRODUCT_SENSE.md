# Product Sense

User experience principles and product thinking for LoongClaw.

## Target Users

LoongClaw is an AI agent runtime. Its users are:

1. **Developers** integrating AI agents into applications via channels (CLI, Telegram, Feishu)
2. **Platform operators** deploying and managing agent runtimes
3. **Plugin authors** extending agent capabilities through tools, connectors, and memory backends

## Product Principles

1. **Safe by default** — No capability is granted without explicit token. New users start in the most restrictive mode.
2. **Progressive disclosure** — Simple things should be simple. Advanced configuration exists but doesn't obstruct the common path.
3. **Transparent execution** — Users can always see what the agent is doing, why it was allowed, and what was denied. The audit trail is the receipt.
4. **Channel-agnostic experience** — Core agent behavior is identical across CLI, Telegram, Feishu. Channel-specific affordances layer on top.
5. **Fail loud, not silent** — Errors surface to the user with actionable context. Silent drops are bugs.

## Product Specifications

See [Product Specs Index](product-specs/index.md) for detailed user-facing requirements:

- [Memory Profiles](product-specs/memory-profiles.md) — memory access patterns
- [Prompt and Personality](product-specs/prompt-and-personality.md) — prompt engineering constraints

## User-Facing Commands

The daemon binary (`loongclaw`) exposes:

| Command | Purpose |
|---------|---------|
| `setup` | First-run configuration |
| `onboard` | Guided onboarding flow |
| `doctor` | Diagnostic health checks |
| `chat` | Interactive CLI conversation |
| `run-spec` | Execute deterministic test specs |

## See Also

- [Roadmap](ROADMAP.md) — stage-based milestones with user impact
- [Contributing](../CONTRIBUTING.md) — how to add channels, tools, providers
