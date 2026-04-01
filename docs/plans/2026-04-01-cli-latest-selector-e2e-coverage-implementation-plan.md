# CLI Latest Selector End-to-End Coverage Implementation Plan

Date: 2026-04-01
Issue: `#759`
PR: `#765`

## Task 1: Add failing startup-summary coverage

**Files:**
- Modify: `crates/app/src/chat.rs`

**Steps:**
1. Seed at least one resumable root session in sqlite memory.
2. Initialize the real CLI runtime with `session_hint = Some("latest")`.
3. Build the startup summary from that runtime.
4. Assert the summary exposes the resolved session id instead of the literal selector token.

**Validation:**
```bash
cargo test -p loongclaw-app cli_runtime_latest_session_selector_updates_startup_summary --locked
```

Expected: failing before the new coverage exists.

## Task 2: Add failing downstream history coverage

**Files:**
- Modify: `crates/app/src/chat.rs`

**Steps:**
1. Seed multiple sessions, including one newest resumable root session and at least one distractor.
2. Initialize the real CLI runtime with `session_hint = Some("latest")`.
3. Load history lines using the runtime's resolved session id.
4. Assert the returned history matches only the newest resumable root session.

**Validation:**
```bash
cargo test -p loongclaw-app cli_runtime_latest_session_selector_drives_history_loads --locked
```

Expected: failing before the new coverage exists.

## Task 3: Implement the minimal supporting change if needed

**Files:**
- Modify only if the new tests expose a real gap

**Steps:**
1. Diagnose whether the failure is in selector resolution, summary propagation, or history loading.
2. Apply the smallest ownership-preserving fix.
3. Avoid broad refactors or new abstractions.

## Task 4: Run focused and broad verification

**Files:**
- Modify: none unless validation exposes a necessary fix

**Commands:**
```bash
cargo fmt --all -- --check
cargo test -p loongclaw-app cli_runtime_latest_session_selector_updates_startup_summary --locked
cargo test -p loongclaw-app cli_runtime_latest_session_selector_drives_history_loads --locked
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: all green.
