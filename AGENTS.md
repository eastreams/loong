# LoongClaw Agent Guide

This document is intentionally mirrored in `CLAUDE.md` and `AGENTS.md`.

This file is the **map** — keep it short (~100 lines). Deeper context lives in `docs/`.

## 1. Start Here

- [Core Beliefs](docs/design-docs/core-beliefs.md) — kernel and engineering principles
- [Layered Kernel Design](docs/design-docs/layered-kernel-design.md) — layered model and boundary rules
- [Roadmap](docs/roadmap.md) — stage-based milestones and acceptance criteria
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

## 3. Commands

- Format check: `cargo fmt --all -- --check`
- Strict lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Test all features: `cargo test --workspace --all-features`
- Canonical verify: `task verify`
- Extended verify: `task verify:full`

## 4. Non-Negotiable Rules

- Kernel contracts are backward-compatible. No breaking changes without documented decision.
- All execution paths route through kernel capability/policy/audit. No shadow paths.
- Strict lint and all-feature tests pass at every commit.
- Never commit credentials, tokens, or private endpoints.
- Keep `CLAUDE.md` and `AGENTS.md` mirrored in the same change.
- **Before every commit**, run CI-parity checks. Any manual edit after fmt must be re-checked.

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

Runs fmt + clippy before each commit, matching CI exactly.

## 7. Where to Look Next

| Need | Go to |
|------|-------|
| Core principles | `docs/design-docs/core-beliefs.md` |
| Layered architecture | `docs/design-docs/layered-kernel-design.md` |
| Roadmap | `docs/roadmap.md` |
| Reliability invariants | `docs/RELIABILITY.md` |
| Product requirements | `docs/product-specs/` |
| Contributing recipes | `CONTRIBUTING.md` |
