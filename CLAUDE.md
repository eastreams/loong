# LoongClaw Agent Guide

This document is intentionally mirrored in `CLAUDE.md` and `AGENTS.md`.

This file is the **map** — keep it short (~100 lines). Deeper context lives in `docs/`.

## 1. Start Here

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
├── spec → contracts, kernel, protocol (+ app: known deviation, tracked as D1)
├── bench → contracts, kernel, spec
└── daemon (binary) → all of the above
```

Non-negotiable: no dependency cycles. See [Core Beliefs](docs/design-docs/core-beliefs.md).
Current tracked deviation: D1 keeps `spec -> app` temporary and must be retired by architectural
refactor, not normalized as permanent layering.

## 3. Commands

- Format check: `cargo fmt --all -- --check`
- Strict lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Architecture check: `task check:architecture`
- Convention check: `task check:conventions`
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
- Every released version must map to `docs/releases/vX.Y.Z.md` with process log and detail links.
- Local agent debug context for a release should be recorded in `.docs/releases/vX.Y.Z-debug.md`.
- Agent execution plans go in `docs/exec-plans/active/` (local only, not committed). Move to `completed/` when done.

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
Use `task verify` for the stricter local superset (architecture, conventions, docs, deny).

## 7. Harness Maintenance

Skills are at `skills/*.skill` (zip archives). If not installed, extract and install for your platform.
Example: `unzip skills/update-harness.skill -d ~/.claude/skills/update-harness/`
Run the `update-harness` skill before pushing to verify mirrors, docs, and quality scores.

## 8. Pitfalls

- No `unwrap()`, `expect()`, `panic!()`, `todo!()`, `[]` indexing, or `println!` — use `?`, `.get()`, and proper logging. The workspace denies all of these at compile time.
- Policy engine only gates `shell.exec` — `file.read`/`file.write` bypass it entirely (TD-002). Don't assume all tools are policy-checked.
- `membrane` field on CapabilityToken is never checked. `call_depth` is always 0. Don't build on stubs.
- `spec -> app` dependency (D1) is a tracked deviation, not permanent. Don't add more cross-layer deps.
- App layer uses `Result<T, String>` (`CliResult`), not typed errors. Kernel uses `thiserror` enums.
- Audit events are in-memory only — lost on restart (TD-006). No persistence yet.
- `BTreeMap` not `HashMap` everywhere — deterministic ordering is intentional.
- `docs/exec-plans/` (agent working memory) and `docs/plans/` (durable phase plans) are local only — not committed to git.

## 9. Where to Look Next

| Need | Go to |
|------|-------|
| Core principles | `docs/design-docs/core-beliefs.md` |
| Layered architecture | `docs/design-docs/layered-kernel-design.md` |
| Design decisions, patterns & catalog | `docs/design-docs/index.md` |
| Harness engineering | `docs/design-docs/harness-engineering.md` |
| Roadmap | `docs/ROADMAP.md` |
| Reliability invariants | `docs/RELIABILITY.md` |
| Security model & gaps | `docs/SECURITY.md` |
| Quality scores & gaps | `docs/QUALITY_SCORE.md` |
| Tech debt tracker (22 items) | `docs/tech-debt-tracker.md` |
| Agent execution plans (local only) | `docs/exec-plans/active/` |
| Durable phase plans (local only) | `docs/plans/` |
| Product sense & principles | `docs/PRODUCT_SENSE.md` |
| Release process docs | `docs/releases/` |
| Product requirements | `docs/product-specs/` |
| LLM-optimized references | `docs/references/` |
| Auto-generated artifacts | `docs/generated/` |
| Agent skills | `skills/*.skill` |
| Contributing recipes | `CONTRIBUTING.md` |
