# Contributing to LoongClaw

Thanks for spending time on LoongClaw.

This file is the repository-native contributor guide. The shorter public
contributor entrypoint lives under
[`site/build-on-loongclaw/contributing.mdx`](site/build-on-loongclaw/contributing.mdx).
This guide stays in the repository because contributors and maintainers still
need the deeper workflow, validation, and recipe material behind that public
entrypoint.

We care about clear ownership, thoughtful engineering, and kind collaboration.

## Read This Guide When

- you are preparing a real patch against this repository
- you need the repository-native validation, branch, or PR rules
- you want crate-level contribution recipes instead of the shorter public docs
  summary

## Choose A Start Path

| If you want to... | Start here | Then continue to... |
| --- | --- | --- |
| land a small docs, test, or contained bug-fix patch | [Contribution Workflow](site/build-on-loongclaw/contribution-workflow.mdx) and [Contribution Tracks](#contribution-tracks) | [Standard Workflow](#standard-workflow) |
| improve docs placement, Mintlify structure, or public docs wording | [Docs Workflow](site/build-on-loongclaw/docs-workflow.mdx) | [Documentation Language Scope](#documentation-language-scope) and [Standard Workflow](#standard-workflow) |
| change runtime behavior, policy, or architecture-sensitive code | [Architecture](site/build-on-loongclaw/architecture.mdx) | [Contribution Tracks](#contribution-tracks), [CI And Required Checks](#ci-and-required-checks), and [Repository Recipes](#repository-recipes) |
| understand where your background is most useful | [Contribution Areas We Especially Welcome](docs/references/contribution-areas.md) | [How To Join In](#how-to-join-in) |
| read the full repository-native contributor guide directly | this file | the [Section Map](#section-map) and the sections below |

## Quick Start Checklist

1. Branch from `dev`.
2. Decide whether the work is Track A or Track B.
3. Run the relevant validation bar before opening a PR.
4. Open the PR against `dev`.
5. Start with an issue or discussion first if the work is large, risky, or architecture-sensitive.

## Section Map

- [Core Workflow And Validation](#core-workflow-and-validation)
- [Starting Areas And Contribution Scope](#starting-areas-and-contribution-scope)
- [Repository Recipes](#repository-recipes)
- [Agent-Assisted Work And Observability](#agent-assisted-work-and-observability)
- [PR, Review, And Security Boundaries](#pr-review-and-security-boundaries)

## What Stays Here

This guide intentionally carries the deeper repository-native contributor
material:

- branch, release, CI, and review expectations
- source-level contribution tracks and validation rules
- crate-level contribution recipes
- responsible agent-assisted contribution guidance
- repository observability and maintainer-facing contribution boundaries

It stays single-file on purpose so contributors can scan one repository-native
guide without chasing a split maintainer handbook. It is not meant to replace
the shorter Mintlify contributor entrypoint.

## Core Workflow And Validation

### Environment Prerequisites

- Rust stable toolchain installed.
- `cargo` available in shell.
- `task` CLI installed (`go-task`) if you want to use the convenience wrappers in
  `Taskfile.yml` such as `task verify` / `task verify:full`.
- Go toolchain installed (required by `task check:conventions`).
- `cargo-deny` installed (required by `task check:deny`).
- GitHub account with fork access.
- Convention checks require the `convention-engineering` skill script at
  `~/.claude/skills/convention-engineering/scripts/main.go` (see `Taskfile.yml`).

### Contribution Tracks

LoongClaw uses two tracks for OSS contribution risk.

### Track A: Routine and low-risk changes

Use Track A for:
- docs updates
- tests
- small refactors
- contained bug fixes

Required checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Optional convenience wrapper:

```bash
task verify
```

If `task` or its transitive dependencies are unavailable locally, run at least
CI parity plus architecture/dep-graph checks directly:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --all-features
scripts/check_architecture_boundaries.sh
scripts/check_dep_graph.sh
```

### Track B: Higher-risk changes

Use Track B for:
- security-sensitive behavior
- API contract changes
- runtime/kernel policy changes
- architecture-impacting refactors

Track B flow:
1. Open an issue or PR draft with design intent first.
2. Wait for maintainer acknowledgement before deep implementation.
3. Run the same baseline checks as Track A plus any scenario/benchmark checks relevant to changed modules.

If you are unsure which track applies, open an issue and ask maintainers for triage.

### Branch Model

- `dev` is the active integration branch for day-to-day development.
- Contributors should branch from `dev` and target `dev` with normal pull requests.
- `main` is the stable promotion branch and should only receive reviewed changes from `dev`.
- `release` or `release/*` branches are reserved for release hardening. When maintainers use one,
  it should only receive reviewed changes from `main`.
- Maintainers aim to promote stable slices from `dev` into `main` on a regular cadence. Exact
  timing depends on validation status, scope completion, and operational readiness.

### Release Model

- Tagged releases are published from stable promotion points rather than from arbitrary in-flight
  commits.
- Maintainers may use `release` or `release/*` branches as short-lived release hardening lanes
  before tagging. Those branches should stay focused on release readiness, fixes, and verification.
- Not every `dev -> main` promotion needs to become a public release.
- Release readiness normally includes green CI, required validation, install flow sanity, and docs
  or changelog updates for shipped user-facing changes.

### CI And Required Checks

- `CI`, `CodeQL`, and `Security` run for pull requests and pushes targeting `dev`, `main`,
  `release`, and `release/*`.
- `perf-lint` follows the same branch set but only when workflow, benchmark, daemon, spec, kernel,
  or app paths change.
- The aggregate required check for promotion branches is `build`, emitted by
  `.github/workflows/ci.yml`.
- If branch protection is enabled for `dev`, `main`, or `release` lanes, require `build` instead
  of tracking the internal job names individually.

### Standard Workflow

1. Fork the repository.
2. Create a branch from `dev`.
3. Make focused commits.
4. Run required checks.
5. Open a pull request against `dev` using the PR template unless a maintainer explicitly asks you
   to help with a focused promotion PR from `dev` into `main`.
6. Address review feedback and keep PR scope focused.

### Issue Intake

- Use the bug report form for reproducible runtime or workflow defects.
- Use the feature request form for new capabilities, behavior changes, or meaningful product/runtime improvements.
- Use the documentation improvement form for contributor guide drift, missing references, or confusing review workflow docs.
- Use GitHub Discussions for setup questions and general troubleshooting.
- Use community channels such as Discord and Telegram. If you are already active in Feishu or
  WeChat community spaces, those are also good places to ask.
- If you want to introduce yourself directly or talk about where you could help most, email
  [contact@loongclaw.ai](mailto:contact@loongclaw.ai).
- Use the private security advisory flow for vulnerabilities instead of public issues.

The public workflow in this guide is the contributor-facing source of truth.
Maintainer-managed GitHub label automation, intake wiring, and branch-governance
support docs remain repository-native support material and do not need to be part
of the normal reader path.

## Starting Areas And Contribution Scope

### Where To Start

Use [Core Beliefs](docs/design-docs/core-beliefs.md) and [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) for architecture principles and dependency boundaries.

If you are unsure where your background fits, start with
[Contribution Areas We Especially Welcome](docs/references/contribution-areas.md). We warmly
welcome help across design, frontend work, hardware / robotics / embodied AI, systems engineering,
cross-platform delivery, testing and operations, docs and public docs-site clarity, and community
care.

### How To Join In

- If you already know what you want to work on, open or join the relevant Issue and link your plan.
- If you want to take on a large feature or architecture change, start with an Issue or Discussion
  first so maintainers can help shape scope early.
- If your strengths are design, docs, docs-site editing, QA, operations, support, or community work,
  those are first-class contributions here, not second-tier work.
- If you would rather start with a direct introduction, email
  [contact@loongclaw.ai](mailto:contact@loongclaw.ai). A short note is enough. You do not need a
  formal application.
- If you are unsure where to begin, open a Discussion or send that introduction email and we will
  help point you toward good starting areas.

### Documentation Language Scope

- The repository keeps Simplified Chinese support only for `README.zh-CN.md`.
- The Mintlify source under `site/` is the main English reader-facing documentation surface.
- Public markdown under `docs/` remains in the repository as supporting reference and source-facing
  documentation unless maintainers intentionally add a broader docs-site workflow later.
- If broader docs i18n is introduced in the future, it should happen at the Mintlify docs layer
  rather than by expanding repository-wide markdown translation.

### A Short Introduction That Helps

If you email us, it is especially helpful to include:

- where you are based or what time zone you usually work in
- your strongest skills or the kinds of problems you are best at
- the area you would most like to own or help push forward
- what you hope LoongClaw could become, or what part of the project excites you
- roughly how much time or energy you expect to contribute
- any links to GitHub, past work, writing, design, demos, or projects you want us to see

That does not need to be long. A thoughtful, honest introduction is much more useful than a formal
pitch.

## Repository Recipes

This section keeps the source-level repository recipes that are too specific
for the shorter public contributor docs entrypoint.

### Common Repository Lanes

| Area | Directory | Feature flag |
|------|-----------|-------------|
| Add a provider | `crates/app/src/provider/` | `provider-openai` |
| Add a tool | `crates/app/src/tools/` | `tools-shell`, `tools-file` |
| Add a channel | `crates/app/src/channel/` | `channel-telegram`, `channel-feishu`, `channel-matrix` |
| Add a memory backend | `crates/app/src/memory/` | `memory-sqlite` |
| Kernel policy | `crates/kernel/src/policy.rs` | — |
| Shared types | `crates/contracts/src/` | — |

### How To Run Tests For Your Module

```bash
# All tests
cargo test --workspace

# Just the mvp crate
cargo test -p loongclaw-app

# Just kernel tests
cargo test -p loongclaw-kernel

# With all features (CI gate)
cargo test --workspace --all-features
```

### Recipe: Add a Provider

1. Start from `crates/app/src/config/provider.rs` and add or update the static provider descriptor facts first:
   canonical id, aliases, protocol family, auth defaults, and setup-facing defaults
2. Update `crates/app/src/provider/contracts.rs` only for request-time behavior that genuinely belongs in the runtime contract:
   transport mode, payload adaptation, validation rules, capability defaults, or error classification
3. Extend the relevant runtime modules such as `transport.rs`, `provider_validation_runtime.rs`, or request/runtime helpers only after the descriptor and runtime contract seams are clear
4. Add or extend provider-family conformance and regression tests so descriptor facts, runtime-contract derivation, and setup/auth guidance stay aligned
5. If the provider needs a feature flag, add it to `crates/app/Cargo.toml`
6. Read [SDK Docs](docs/sdk/index.md) and the provider convergence plan before large provider-family refactors

### Recipe: Add a Tool

1. Create `crates/app/src/tools/your_tool.rs`
2. Add a handler function: `pub fn execute_your_tool(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String>`
3. Add a match arm in `execute_tool_core()` in `crates/app/src/tools/mod.rs`
4. The tool automatically routes through the kernel when `KernelContext` is present (policy + audit)
5. Add tests — the kernel integration is already wired via `MvpToolAdapter`

### Recipe: Add a Channel

1. Create `crates/app/src/channel/your_channel/mod.rs`
2. Implement the `ChannelAdapter` trait (`name`, `receive_batch`, `send_message`)
3. Add a `run_your_channel()` function in `crates/app/src/channel/mod.rs` that:
   - Loads config
   - Calls `bootstrap_kernel_context_with_config("channel-your-channel", DEFAULT_TOKEN_TTL_S, &config)`
   - Loops: receive messages → `process_inbound_with_provider(config, msg, Some(&ctx))` → send reply
4. Wire the subcommand in `crates/daemon/src/main.rs`
5. Add a feature flag in `crates/app/Cargo.toml`

The shipped channel reference implementations are `telegram`, `feishu`, and `matrix`.

## Agent-Assisted Work And Observability

### Responsible Agent-Assisted Contribution

LoongClaw is built with the expectation that human engineers and agents will increasingly work
together. We think that trend is real, durable, and worth embracing. Used well, agent-assisted
coding can significantly improve iteration speed, reduce routine friction, and help contributors
cover more ground across design, implementation, testing, docs, and review.

That does **not** reduce the contributor's responsibility. We expect every author to understand the
changes they submit, to be able to explain why they made them, and to own the resulting behavior,
risks, and tradeoffs. "The agent wrote it" is never a sufficient reason for surprising behavior,
unclear code, missing validation, or low-confidence changes.

Our stance is therefore not "no AI" and not "anything goes." We support **responsible use of
agents**:

- use agents to accelerate research, editing, implementation, tests, and review preparation
- keep a human in the loop for architectural judgment, validation, and final submission decisions
- verify generated changes before opening a PR, especially when touching runtime behavior, policy,
  security, or contributor-facing docs
- prefer smaller, reviewable diffs over large opaque drops that nobody on the PR can confidently
  defend
- call out important agent assistance in the PR when it materially affected scope, verification, or
  risk

We are also actively investing in **harness engineering** inside this repository. That means we
want better contributor workflows, stronger observability, more reliable validation paths, and
clearer scaffolding for people who use agents to contribute. Expect the repository's agent-facing
recipes, diagnostics, and guardrails to keep improving over time, and feel free to contribute to
that work directly.

If you like "vibe coding," use it as a way to explore and iterate faster, not as a substitute for
engineering accountability. The bar here is still that a human contributor understands what is
being merged and can stand behind it during review, release, and follow-up maintenance.

### Developer Observability

When you want an agent to help debug a repository issue or prepare review
findings, start from the built-in observability surfaces instead of external
skill setup:

```bash
loong doctor --config ~/.loongclaw/config.toml
loong doctor --config ~/.loongclaw/config.toml --json
loong audit recent --config ~/.loongclaw/config.toml
loong audit summary --config ~/.loongclaw/config.toml
loong audit recent --config ~/.loongclaw/config.toml --json
if [ -f ~/.loongclaw/audit/events.jsonl ]; then tail -n 20 ~/.loongclaw/audit/events.jsonl; else echo "audit journal is created on first audit write"; fi
```

The app runtime defaults to durable audit retention with
`[audit].mode = "fanout"`, so security-critical audit events persist across
restarts under `~/.loongclaw/audit/events.jsonl`. Use `doctor --fix` if you
want LoongClaw to pre-create the audit journal directory before a debugging
session. Reach for `audit recent` when you need the latest bounded event window
and `audit summary` when you need a quick rollup before diving into raw JSONL.

For Rust workspaces, keep one agent per worktree or target directory so cargo
lock contention does not invalidate the debugging signal.

## PR, Review, And Security Boundaries

### PRs We Are Unlikely To Merge

The following pull requests are unlikely to be accepted unless maintainers have explicitly aligned
on them in advance:

1. AI-assisted changes that the author does not understand or cannot defend. We welcome AI tooling
   as a force multiplier, but contributors are expected to understand every line they submit and to
   take responsibility for its behavior, risks, and tradeoffs.
2. Uncoordinated changes to core project identity or governance files. This includes brand assets,
   licensing, pull request templates, `CODEOWNERS`, and similar repository-critical configuration.
   These areas are maintained by the core team and may be closed without review if changed without
   prior discussion.
3. Large changes to core product architecture without prior maintainer discussion. If you want to
   build a major feature or restructure key architecture, start with an Issue or Discussion, align
   on the design, and wait for maintainer guidance before implementation.
4. Ecosystem extensions outside the core product scope. Third-party plugins, external integrations,
   and ecosystem-specific additions are usually better maintained in separate repositories instead
   of being merged into the main repository.

### Commit And PR Expectations

- Use clear, scoped commit messages.
- Keep one logical change per PR when possible.
- Link relevant issue IDs in PR description.
- When a PR resolves a tracked issue, include an explicit closing clause such as `Closes #123` in
  the PR body.
- Include risk notes for Track B changes.
- Promotion PRs from `dev` into `main` should stay narrow and focus on stabilization rather than
  mixed feature development.

### Review Policy

- At least one maintainer review is required.
- Track B changes require explicit maintainer approval.
- Maintainers may request design clarification before merge.

### Reporting Security Issues

Do not open public issues for security vulnerabilities. Follow [SECURITY.md](SECURITY.md).

### Do Not Use This Guide For

- maintainer-only GitHub intake automation details that belong in
  `docs/references/github-collaboration.md`
- public docs landing-page reading that is already covered by Mintlify under
  `site/`
- internal planning bundles or private governance material that does not belong
  in the OSS repository
