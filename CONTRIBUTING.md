# Contributing to LoongClaw

Thanks for contributing. This guide defines the baseline workflow for external and internal contributors.

## Prerequisites

- Rust stable toolchain installed.
- `cargo` available in shell.
- `task` CLI installed (`go-task`), required for `task verify` / `task verify:full`.
- Go toolchain installed (required by `task check:conventions`).
- `cargo-deny` installed (required by `task check:deny`).
- GitHub account with fork access.
- Convention checks require the `convention-engineering` skill script at
  `~/.claude/skills/convention-engineering/scripts/main.go` (see `Taskfile.yml`).

## Contribution Tracks

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

Canonical local gate:

```bash
task verify
```

If `task`/`go`/convention skill dependencies are unavailable locally, run at least CI parity plus
architecture/dep-graph checks directly:

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

## Branch Model

- `alpha-test` is the active integration branch for normal OSS work.
- `main` is the promotion branch and should only receive reviewed changes from `alpha-test`.
- The repository default branch setting does not yet fully reflect this flow. Until that changes, treat the docs and workflow files on `alpha-test` as the authoritative contributor baseline.

## Where Do I Start?

Use [Core Beliefs](docs/design-docs/core-beliefs.md) and [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) for architecture principles and dependency boundaries.

**Common contribution areas:**

| Area | Directory | Feature flag |
|------|-----------|-------------|
| Add a provider | `crates/app/src/provider/` | `provider-openai` |
| Add a tool | `crates/app/src/tools/` | `tools-shell`, `tools-file` |
| Add a channel | `crates/app/src/channel/` | `channel-telegram`, `channel-feishu` |
| Add a memory backend | `crates/app/src/memory/` | `memory-sqlite` |
| Kernel policy | `crates/kernel/src/policy.rs` | — |
| Shared types | `crates/contracts/src/` | — |

### How to Run Tests for Your Module

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

1. Copy `crates/app/src/provider/transport.rs` as a starting point
2. Implement the HTTP transport for your provider's API
3. Update `crates/app/src/provider/mod.rs` to route to your provider based on config
4. Add tests in the same file
5. If the provider needs a feature flag, add it to `crates/app/Cargo.toml`

### Recipe: Add a Tool

1. Create `crates/app/src/tools/your_tool.rs`
2. Add a handler function: `pub fn execute_your_tool(request: ToolCoreRequest) -> Result<ToolCoreOutcome, String>`
3. Add a match arm in `execute_tool_core()` in `crates/app/src/tools/mod.rs`
4. The tool automatically routes through the kernel when `KernelContext` is present (policy + audit)
5. Add tests — the kernel integration is already wired via `MvpToolAdapter`

### Recipe: Add a Channel

1. Create `crates/app/src/channel/your_channel/mod.rs`
2. Implement the `ChannelAdapter` trait (`name`, `receive_batch`, `send_text`)
3. Add a `run_your_channel()` function in `crates/app/src/channel/mod.rs` that:
   - Loads config
   - Calls `bootstrap_kernel_context("channel-your-channel", DEFAULT_TOKEN_TTL_S)`
   - Loops: receive messages → `process_inbound_with_provider(config, msg, Some(&ctx))` → send reply
4. Wire the subcommand in `crates/daemon/src/main.rs`
5. Add a feature flag in `crates/app/Cargo.toml`

## Standard Workflow

1. Fork the repository.
2. Create a branch from `alpha-test`.
3. Make focused commits.
4. Run required checks.
5. Open a pull request against `alpha-test` using the PR template unless a maintainer explicitly asks for a promotion PR into `main`.
6. Address review feedback and keep PR scope focused.

## Issue Intake

- Use the bug report form for reproducible runtime or workflow defects.
- Use the feature request form for new capabilities, behavior changes, or meaningful product/runtime improvements.
- Use the documentation improvement form for contributor guide drift, missing references, or confusing review workflow docs.
- Use GitHub Discussions for setup questions and general troubleshooting.
- Use the private security advisory flow for vulnerabilities instead of public issues.

See [docs/references/github-collaboration.md](docs/references/github-collaboration.md) for the current label baseline, issue routing, and the default-branch visibility caveat for issue forms.

## Commit and PR Expectations

- Use clear, scoped commit messages.
- Keep one logical change per PR when possible.
- Link relevant issue IDs in PR description.
- Include risk notes for Track B changes.

## Review Policy

- At least one maintainer review is required.
- Track B changes require explicit maintainer approval.
- Maintainers may request design clarification before merge.

## Reporting Security Issues

Do not open public issues for security vulnerabilities. Follow [SECURITY.md](SECURITY.md).
