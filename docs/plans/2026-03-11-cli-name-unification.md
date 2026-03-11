# CLI Name Unification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `loongclaw` the only CLI command name on `alpha-test`, with no
legacy daemon-suffixed compatibility path.

**Architecture:** Rename the binary target and clap identity first, then sweep
all operator-facing references in source, scripts, docs, examples, and release
workflow configuration so the repository exposes one command name end-to-end.

**Tech Stack:** Rust, clap, shell, PowerShell, GitHub Actions, Markdown

---

### Task 1: Add failing tests for the canonical CLI name

**Files:**
- Modify: `crates/daemon/src/tests/mod.rs`
- Modify: `crates/app/src/config/runtime.rs`

Add tests that prove:

- the clap command name is `loongclaw`
- the config-load guidance tells users to run `loongclaw setup`

### Task 2: Rename the binary and source-level command guidance

**Files:**
- Modify: `crates/daemon/Cargo.toml`
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/app/src/config/runtime.rs`

Rename the compiled binary and every built-in operator hint from the old
daemon-suffixed command name to `loongclaw`.

### Task 3: Update install, documentation, examples, and release workflow

**Files:**
- Modify: `scripts/install.sh`
- Modify: `scripts/install.ps1`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `examples/README.md`
- Modify: `ARCHITECTURE.md`
- Modify: `Taskfile.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `docs/plans/*.md` entries that still show the old command name where
  the current branch should describe the new canonical CLI

Remove the old daemon-suffixed command name from repository-owned operator
instructions and build automation.

### Task 4: Verify the rename

Run:

```bash
cargo test -p loongclaw-app config::
cargo test -p loongclaw-daemon tests::
rg -n '\bloongclawd\b' .
cargo test --workspace --all-features
```
