# Issue 400 Checklist

- [x] Add tests for bare `loongclaw` no-args routing when config is missing vs present.
- [x] Add tests for the welcome banner content and command hints.
- [x] Implement a default entry resolver and a new `welcome` command path.
- [x] Switch `main` to use the new default entry behavior while keeping `demo` explicit.
- [x] Run targeted daemon tests during red/green cycles.
- [x] Run `task verify`.

## Progress

- 2026-03-20: Created isolated product worktree `/Users/xj/github/loongclaw/loongclaw/.worktrees/issue-400-default-welcome` from `upstream/dev` for issue #400.
- 2026-03-20: Confirmed the current no-args path still defaults to `Commands::Demo` in `crates/daemon/src/main.rs`.
- 2026-03-20: Confirmed canonical config path resolution lives in `crates/app/src/config/runtime.rs` and existing setup-next-action wording lives in `crates/daemon/src/next_actions.rs`.
- 2026-03-20: Added red tests for the new welcome subcommand help, no-args resolver, and welcome banner output; initial compile failed as expected because `Welcome`, the resolver, and the banner renderer did not exist yet.
- 2026-03-20: Implemented `Commands::Welcome`, `resolve_default_entry_command()`, `run_welcome_cli()`, and the no-args handoff in `crates/daemon/src/main.rs`.
- 2026-03-20: Green tests passed for `first_run_entry` and `welcome_subcommand_help_advertises_first_run_shortcuts`.
- 2026-03-21: `task verify` initially failed on a pre-existing `cargo deny` advisory for `rustls-webpki 0.103.9`; updated `Cargo.lock` to `rustls-webpki 0.103.10`, then reran `task verify` successfully.

## Review / Results

- Behavior change: bare `loongclaw` now routes to interactive onboarding when no config exists, and to a new `welcome` banner when config is already present.
- Explicit `loongclaw demo` remains available; only the no-args default changed.
- Verification:
  - `cargo test -p loongclaw-daemon first_run_entry -- --nocapture`
  - `cargo test -p loongclaw-daemon welcome_subcommand_help_advertises_first_run_shortcuts -- --nocapture`
  - `task verify`
