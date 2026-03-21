# Issue 400 Checklist

- [x] Add tests for bare `loongclaw` no-args routing when config is missing vs present.
- [x] Add tests for the welcome banner content and command hints.
- [x] Implement a default entry resolver and a new `welcome` command path.
- [x] Switch `main` to use the new default entry behavior while keeping `demo` explicit.
- [x] Run targeted daemon tests during red/green cycles.
- [x] Run `task verify`.
- [x] Address actionable PR review feedback on `welcome` command behavior and committed docs.
- [x] Add regression coverage for explicit `welcome` behavior, config-file detection, and test temp-dir uniqueness.
- [x] Reply to or resolve PR review threads after verification.

## Progress

- 2026-03-20: Created isolated product worktree `.worktrees/issue-400-default-welcome` from `upstream/dev` for issue #400.
- 2026-03-20: Confirmed the current no-args path still defaults to `Commands::Demo` in `crates/daemon/src/main.rs`.
- 2026-03-20: Confirmed canonical config path resolution lives in `crates/app/src/config/runtime.rs` and existing setup-next-action wording lives in `crates/daemon/src/next_actions.rs`.
- 2026-03-20: Added red tests for the new welcome subcommand help, no-args resolver, and welcome banner output; initial compile failed as expected because `Welcome`, the resolver, and the banner renderer did not exist yet.
- 2026-03-20: Implemented `Commands::Welcome`, `resolve_default_entry_command()`, `run_welcome_cli()`, and the no-args handoff in `crates/daemon/src/main.rs`.
- 2026-03-20: Green tests passed for `first_run_entry` and `welcome_subcommand_help_advertises_first_run_shortcuts`.
- 2026-03-21: `task verify` initially failed on a pre-existing `cargo deny` advisory for `rustls-webpki 0.103.9`; updated `Cargo.lock` to `rustls-webpki 0.103.10`, then reran `task verify` successfully.
- 2026-03-21: After opening PR `#407`, GitHub `advisory-checks` still failed on `cargo audit` for `aws-lc-sys 0.38.0` (`RUSTSEC-2026-0048`, `RUSTSEC-2026-0044`); updated `Cargo.lock` via `aws-lc-rs 1.16.2` -> `aws-lc-sys 0.39.0` and reran local verification before pushing the follow-up.
- 2026-03-21: Reviewed unresolved PR feedback on `#407`; confirmed the actionable items are the absolute path leak in this file, file-vs-directory config detection, explicit `welcome` behavior when config is missing, help-text alignment, and stronger temp-dir uniqueness in tests.
- 2026-03-21: Added red tests for directory config-path handling, explicit `welcome` missing-config failure paths, and `welcome` help-text alignment; confirmed they failed before the fix.
- 2026-03-21: Updated `resolve_default_entry_command()` to require a config file, made `run_welcome_cli()` fail closed with onboarding guidance when config is missing, aligned `welcome` help text with `--config <path>` usage, redacted the committed worktree path, and strengthened test temp-dir uniqueness with PID + atomic counter suffixes.
- 2026-03-21: Re-ran targeted daemon tests plus `task verify`; all local gates passed after the review-fix follow-up.
- 2026-03-21: Pushed review follow-up commit `a2cbbe1`, replied in-thread to the six unresolved PR findings on `#407`, and resolved those review threads after the new head landed.

## Review / Results

- Behavior change: bare `loongclaw` now routes to interactive onboarding when no config exists, and to a new `welcome` banner when config is already present.
- Explicit `loongclaw demo` remains available; only the no-args default changed.
- Verification:
  - `cargo test -p loongclaw-daemon first_run_entry -- --nocapture`
  - `cargo test -p loongclaw-daemon welcome_subcommand_help_advertises_first_run_shortcuts -- --nocapture`
  - `task verify`
  - `cargo audit`
