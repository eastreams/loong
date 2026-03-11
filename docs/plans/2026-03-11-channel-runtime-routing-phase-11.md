# Channel Runtime Routing Phase 11 Implementation Plan

**Goal:** Reuse default-account provenance inside runtime entrypoints so risky
implicit multi-account fallback routing becomes visible at launch time.

### Task 1: Add failing route-context tests

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/channel/mod.rs`

Add tests that prove:

- a resolved route flags implicit multi-account fallback when `--account` is
  omitted
- explicit account selection suppresses that flag
- explicit default and mapped default do not trigger fallback warnings
- runtime warning rendering only emits output in the risky case

### Task 2: Implement shared resolved-route abstraction

**Files:**
- Modify: `crates/app/src/config/channels.rs`
- Modify: `crates/app/src/config/mod.rs`

Implement a reusable account-route view and expose it for Telegram and
Feishu/Lark config resolution.

### Task 3: Wire route context into runtime entrypoints

**Files:**
- Modify: `crates/app/src/channel/mod.rs`
- Modify: `crates/app/src/channel/feishu/mod.rs`

Use the shared route view to:

- emit inline warnings for risky fallback routing
- add route-source metadata to runtime startup / send output

### Task 4: Verification

Run:

```bash
cargo test -p loongclaw-app config::
cargo test -p loongclaw-app channel::
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
