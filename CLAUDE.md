# LoongClaw Agent Guide

This document is intentionally mirrored in `CLAUDE.md` and `AGENTS.md`.

This file is the **map** — keep it short (~100 lines). Deeper context lives in `docs/`.

## 1. Start Here

- [Core Beliefs](docs/design-docs/core-beliefs.md) — 10 golden principles
- [ARCHITECTURE.md](ARCHITECTURE.md) — 7-crate DAG, data flow, "where does X live"
- [Quality Score](docs/QUALITY_SCORE.md) — per-crate grades
- [Docs Index](docs/index.md) — full documentation map

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
| Architectural decisions | `docs/design-docs/` |
| Active implementation plans | `docs/exec-plans/active/` |
| Known tech debt | `docs/exec-plans/tech-debt-tracker.md` |
| Reference material | `docs/references/` |
| Roadmap | `docs/roadmap.md` |
| Reliability invariants | `docs/RELIABILITY.md` |
| Contributing recipes | `CONTRIBUTING.md` |
