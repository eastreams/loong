# Loong Agent Guide

This document is intentionally mirrored in `CLAUDE.md` and `AGENTS.md`.

This file is the **map** — keep it short (~100 lines). Deeper repo-native
context lives in `docs/`, while the public reader-facing docs surface lives in
`site/`.

## 1. Start Here

- [Repository Docs Map](docs/README.md) — audience routing for `site/` vs `docs/`
- [Core Beliefs](docs/design-docs/core-beliefs.md) — kernel and engineering principles
- [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) — layered model and boundary rules
- [Roadmap](docs/ROADMAP.md) — stage-based milestones and acceptance criteria
- [Reliability](docs/RELIABILITY.md) — invariants and operating expectations
- [Product Specs](docs/product-specs/index.md) — user-facing requirements
- [Contributing Guide](CONTRIBUTING.md) — contributor workflow and recipes

## 2. Architecture Contract

```text
contracts (leaf — zero internal deps)
├── kernel → contracts
├── protocol (independent leaf)
├── app → contracts, kernel
├── spec → contracts, kernel, protocol
├── bench → contracts, kernel, spec
└── daemon (binary) → all of the above
```

Non-negotiable: no dependency cycles. See [Core Beliefs](docs/design-docs/core-beliefs.md).
Current tracked deviations: none.

## 3. Commands

`Taskfile.yml` provides optional convenience wrappers through the `task` CLI.
If `task` is unavailable, run the underlying `cargo` and `scripts/*` commands
directly.

- Format check: `cargo fmt --all -- --check`
- Strict lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Architecture check: `./scripts/check_architecture_boundaries.sh` or `task check:architecture`
- Convention check: `task check:conventions` (optional wrapper; requires Go + convention-engineering skill)
- Test all features: `cargo test --workspace --all-features`
- Canonical verify: `task verify` (optional wrapper around repo verification steps)
- Extended verify: `task verify:full` (optional wrapper around the extended local gate)

## 4. Non-Negotiable Rules

- Kernel contracts are backward-compatible. No breaking changes without documented decision.
- All execution paths route through kernel capability/policy/audit. No shadow paths.
- Strict lint and all-feature tests pass at every commit.
- Never commit credentials, tokens, or private endpoints.
- Keep `CLAUDE.md` and `AGENTS.md` mirrored in the same change.
- **Before every commit**, run CI-parity checks. Any manual edit after fmt must be re-checked.
- Every released version must map to `docs/releases/vX.Y.Z.md` with process log and detail links.
- Local agent debug context for a release should be recorded in `.docs/releases/vX.Y.Z-debug.md`.
- Public-repo issues, PRs, and public-doc wording should stay LoongClaw-centric; keep detailed external project comparisons in `loongclaw-ai/knowledge-base` unless naming an external project is strictly necessary.

## 5. Verification Gates

CI enforces:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --all-features`

## 6. Pre-Commit Hook

```bash
cp scripts/pre-commit .git/hooks/pre-commit && chmod +x .git/hooks/pre-commit
```

Runs CI-parity cargo checks before each commit.
Use `task verify` when the `task` CLI is installed. Otherwise run the
underlying `cargo` and `scripts/*` verification commands directly.

## 7. Where to Look Next

| Need | Go to |
|------|-------|
| Architecture overview & crate DAG | `ARCHITECTURE.md` |
| Repository docs layering | `docs/README.md` |
| Core principles | `docs/design-docs/core-beliefs.md` |
| Layered architecture | `docs/design-docs/layered-kernel-design.md` |
| Design decisions, patterns & catalog | `docs/design-docs/index.md` |
| Harness engineering | `docs/design-docs/harness-engineering.md` |
| Roadmap | `docs/ROADMAP.md` |
| Reliability invariants | `docs/RELIABILITY.md` |
| Security model & gaps | `docs/SECURITY.md` |
| Product sense & principles | `docs/PRODUCT_SENSE.md` |
| Release process conventions | `docs/releases/README.md` |
| Product requirements | `docs/product-specs/` |
| Repository support references | `docs/references/README.md` |
| Contributing recipes | `CONTRIBUTING.md` |
