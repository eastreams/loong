# Personalize Clear And Recovery Follow-up Plan

**Goal:** Let operators explicitly clear saved personalize fields and make the suppressed-to-configured recovery path explicit without changing the advisory-only architecture.

**Architecture:** Keep the existing typed `memory.personalization` model and `personalize` command. Fix the missing behavior in the interactive input layer: text fields need a real clear gesture distinct from "keep current", enum fields need an explicit clear choice when a current value exists, and save logic needs to allow clearing previously saved preferences back to no personalization state. Keep suppression advisory-only and make rerunning `personalize` the explicit recovery path.

**Tech Stack:** Rust, existing daemon prompt helpers, daemon unit tests, daemon integration tests, cargo fmt, cargo test, cargo clippy.

---

## Implementation Tasks

### Task 1: Add failing tests for clear and recovery behavior

**Files:**
- Modify: `crates/daemon/src/personalize_cli.rs`

**Step 1: Add a red test for clearing existing text fields**

Add a unit test that starts from a config with saved text preferences, enters `-`
for one or more current text fields, saves, and asserts the cleared fields are
removed from the persisted personalization block.

**Step 2: Add a red test for clearing existing enum fields**

Add a unit test that starts from a config with saved enum preferences, selects
an explicit clear option, saves, and asserts the enum fields are removed from
the persisted personalization block.

**Step 3: Add a red test for clearing all saved preferences**

Add a unit test that starts from a config with saved preferences, clears every
field, saves, and asserts `memory.personalization` is removed entirely instead
of failing with a "requires at least one operator preference" error.

**Step 4: Add a red test for suppressed-state recovery**

Add a unit test that starts from `prompt_state = Suppressed`, reruns
`personalize`, saves at least one preference, and asserts the command persists
`prompt_state = Configured`.

**Step 5: Run the focused test target and verify red**

Run:

```bash
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test -p loongclaw-daemon personalize_cli
```

Expected:
- the new tests fail for the current implementation because current values
  cannot be cleared cleanly and clearing all preferences is rejected

### Task 2: Implement the minimal interactive and persistence changes

**Files:**
- Modify: `crates/daemon/src/operator_prompt.rs`
- Modify: `crates/daemon/src/personalize_cli.rs`

**Step 1: Reuse existing clear-input conventions for text prompts**

Add a small shared helper that lets `personalize` distinguish:

- blank input -> keep current value
- `-` -> clear current value
- non-empty text -> set new value

Keep the implementation aligned with the existing onboard clear-token
convention instead of inventing a new gesture.

**Step 2: Add explicit clear choices for enum prompts**

When `response_density` or `initiative_level` already has a value, append a
clear option to the select list and return `None` when that option is chosen.

Do not add a clear option when no current value exists.

**Step 3: Allow clearing all previously saved preferences**

Adjust save logic so:

- empty draft + no existing preferences still fails
- empty draft + existing saved preferences clears `memory.personalization`
- empty draft + suppressed-only state still stays invalid for save

Do not silently downgrade the memory profile.

**Step 4: Make suppressed-state recovery explicit**

When current state is suppressed, print a short message explaining that saving
preferences here will re-enable personalization and switch back to configured
state.

**Step 5: Keep the code style narrow and local**

Prefer named intermediate variables and direct helper functions over new shared
frameworks or abstraction layers.

### Task 3: Update product-facing docs and verification

**Files:**
- Modify: `docs/product-specs/personalization.md`
- Modify: `crates/daemon/tests/integration/personalize_cli.rs`
- Modify: PR body for `#812`

**Step 1: Update the spec to mention rerun / clear semantics**

Document that operators can rerun `loong personalize` to update or clear
saved preferences and that suppression only hides suggestions until the command
is run explicitly again.

**Step 2: Add or update CLI help coverage if needed**

If help text changes, keep integration help coverage aligned.

**Step 3: Run focused verification**

Run:

```bash
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test -p loongclaw-daemon personalize_cli
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test -p loongclaw-daemon personalize_cli_accepts_config_flag
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test -p loongclaw-daemon cli_personalize_help_mentions_operator_preferences
```

Expected:
- focused personalize behavior and help coverage pass

### Task 4: Run repo verification and update the existing GitHub delivery

**Files:**
- Modify: existing PR `#812` body

**Step 1: Run repo verification**

Run:

```bash
cargo fmt --all -- --check
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo clippy --workspace --all-targets --all-features -- -D warnings
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test --workspace --locked
CARGO_TARGET_DIR=/tmp/loongclaw-target-personalize cargo test --workspace --all-features --locked
scripts/check_architecture_boundaries.sh
scripts/check_dep_graph.sh
diff CLAUDE.md AGENTS.md
scripts/check-docs.sh
```

**Step 2: Commit the follow-up scope cleanly**

Create a focused commit that contains:

- the follow-up plan doc
- personalize clear/recovery code
- matching tests and spec updates

**Step 3: Push and update GitHub artifacts**

Push the existing branch, update PR `#812` body so reviewer guidance and
validation mention clear/recovery behavior, and keep the issue/PR linkage
unchanged instead of opening duplicates.
