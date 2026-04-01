# Bash Exec AST Governance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the next implementation slice for issue `#637`: AST-backed, prefix-rule-based governance for `bash.exec`, with conservative `Default(Allow|Deny)` fallback, without changing `shell.exec`.

**Architecture:** Keep `bash.exec` as the only governed surface in this slice. Resolve the default home rules directory (`~/.loongclaw/rules`) plus compatibility rules translated from `tools.shell_allow` / `tools.shell_deny` into one compiled prefix-rule set. Parse `bash.exec` commands with `tree-sitter-bash`, split only the supported outer list forms (`;`, newline, `&&`, `||`), classify each unit as governable plain command or unsupported structure, aggregate unit results with deny precedence plus `tools.shell_default_mode`, and only execute Bash when the whole command resolves to `Allow`. Keep rule loading fail-closed for broken rule files, but treat a missing rules directory as an empty explicit rule set.

**Tech Stack:** Rust, `tree-sitter` + `tree-sitter-bash`, `starlark`, serde/toml config parsing, existing `bash.exec` executor, Cargo unit tests, existing tool runtime config and tool dispatch infrastructure in `crates/app`.

**Correctness Review Mode:** `auto-fix`

**Style Review Mode:** `single-pass`

---

## File Structure

- Create: `crates/app/src/tools/bash_governance.rs`
  - Top-level evaluator API, decision/result types, whole-command aggregation, structured denial analysis.
- Create: `crates/app/src/tools/bash_rules.rs`
  - Task-1 bridge types for compiled prefix rules and fail-closed directory loading, then Task-2 Starlark-backed rule loading, `~/.loongclaw/rules/*.rules` discovery, and compatibility-rule translation from `shell_allow` / `shell_deny`.
- Create: `crates/app/src/tools/bash_ast.rs`
  - `tree-sitter-bash` parsing, parse-error detection, minimal-command-unit extraction, unsupported-structure classification.
- Modify: `crates/app/Cargo.toml`
  - Add parser/rule-engine dependencies needed by the new governance modules.
- Modify: `crates/app/src/config/tools.rs`
  - Extend `BashToolConfig` with a rules-directory override and resolution helpers; add config parsing tests.
- Modify: `crates/app/src/tools/runtime_config.rs`
  - Add typed runtime config for Bash governance, resolve the default `~/.loongclaw/rules` path, compile rules at runtime-config build time, preserve fail-closed load errors for later execution.
- Modify: `crates/app/src/tools/bash.rs`
  - Run governance evaluation before process execution, surface policy denials, keep the existing successful execution path unchanged.
- Modify: `crates/app/src/tools/catalog.rs`
  - Update direct `BashExecRuntimePolicy` test constructors so intermediate task slices still compile after adding governance fields.
- Modify: `crates/app/src/tools/mod.rs`
  - Register new modules, update direct `BashExecRuntimePolicy` test constructors, and add end-to-end `bash.exec` governance tests via the existing tool-core harness.
- Modify: `Cargo.toml`
  - Only if promoting new shared dependency versions to `[workspace.dependencies]` is cleaner than crate-local declarations.

## Implementation Notes

- This plan implements [`2026-03-29-bash-governance-ast-prefix-rule-design.md`](2026-03-29-bash-governance-ast-prefix-rule-design.md), not the whole of issue `#637`.
- `shell.exec` remains untouched in behavior and policy semantics.
- `approval_required`, `approve_once`, and `approve_always` remain out of scope.
- Default rules directory behavior:
  - if `tools.bash.rules_dir` is configured, resolve and use it
  - otherwise use `~/.loongclaw/rules`
- Missing rules directory is not an error; broken `.rules` files are an error.
- Compatibility translation:
  - `shell_allow = ["cargo"]` becomes allow prefix `["cargo"]`
  - `shell_deny = ["rm"]` becomes deny prefix `["rm"]`
- Prefix-rule token matching is lexical:
  - plain quoting and escaping are resolved into argv text before matching
  - unsupported shell structures never reach the governable argv path
- Deny precedence must hold regardless of rule source.
- Parse trees with errors or missing nodes must fall back to whole-command `Default`.
- Only the supported outer splitting forms are decomposed in this slice:
  - `;`
  - newline lists
  - `&&`
  - `||`
- Unsupported unit structures in this slice:
  - env-prefix assignments
  - redirections / heredocs / herestrings
  - pipelines
  - subshells
  - command substitution / process substitution
  - functions / loops / conditional compounds

## Scope-Out Follow-up

- Do not add `approval_required` semantics to Bash governance in this slice.
- Do not add `shell.exec` reuse or convergence work in this slice.
- Do not add regex, wildcard, or AST predicate rules in this slice.
- Do not invent partial semantics for unsupported Bash structures by “downgrading” them to plain commands.

### Task 1: Add config, runtime, and minimal rule-type scaffolding

**Files:**
- Modify: `crates/app/Cargo.toml`
- Create: `crates/app/src/tools/bash_rules.rs`
- Modify: `crates/app/src/config/tools.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`
- Modify: `crates/app/src/tools/bash.rs`
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/config/tools.rs`
- Test: `crates/app/src/tools/runtime_config.rs`

- [x] **Step 1: Add red tests for `tools.bash.rules_dir` parsing and default resolution**

Add config tests in `crates/app/src/config/tools.rs`:

```rust
#[test]
#[cfg(feature = "config-toml")]
fn tool_config_parses_bash_rules_dir_override() {
    let config: ToolConfig = toml::from_str("[bash]\nrules_dir = \"custom/rules\"\n")
        .expect("bash tool config");
    assert_eq!(config.bash.rules_dir.as_deref(), Some("custom/rules"));
}

#[test]
fn bash_tool_config_defaults_to_no_explicit_rules_dir() {
    let config = BashToolConfig::default();
    assert!(config.rules_dir.is_none());
}
```

Add runtime-config tests in `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn tool_runtime_config_uses_default_home_rules_dir_when_unset() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut env = ScopedEnv::new();
    env.set("HOME", home.path());
    let loongclaw = crate::config::LoongClawConfig::default();
    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &loongclaw,
        Some(std::path::Path::new("/tmp/work/loongclaw.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        crate::config::default_loongclaw_home().join("rules")
    );
}

#[test]
fn tool_runtime_config_projects_bash_rules_dir_override() {
    let tools: crate::config::ToolConfig =
        toml::from_str("[bash]\nrules_dir = \"custom/rules\"\n").expect("bash tool config");
    let loongclaw = crate::config::LoongClawConfig {
        tools,
        ..crate::config::LoongClawConfig::default()
    };

    let runtime = ToolRuntimeConfig::from_loongclaw_config(
        &loongclaw,
        Some(std::path::Path::new("/tmp/work/loongclaw.toml")),
    );

    assert_eq!(
        runtime.bash_exec.governance.rules_dir,
        std::path::PathBuf::from("/tmp/work/custom/rules")
    );
}
```

- [x] **Step 2: Add red tests for missing-dir vs broken-rule-file runtime state**

Add runtime-config tests in `crates/app/src/tools/runtime_config.rs`:

```rust
#[test]
fn bash_governance_runtime_treats_missing_rules_dir_as_empty_rule_set() {
    let root = tempfile::tempdir().expect("tempdir");
    let config_path = root.path().join("loongclaw.toml");
    let loongclaw = crate::config::LoongClawConfig::default();

    let runtime = ToolRuntimeConfig::from_loongclaw_config(&loongclaw, Some(&config_path));

    assert!(runtime.bash_exec.governance.load_error.is_none());
    assert!(runtime.bash_exec.governance.rules.is_empty());
}

#[test]
fn bash_governance_runtime_preserves_rule_load_error_for_broken_rule_file() {
    let root = tempfile::tempdir().expect("tempdir");
    let rules_dir = root.path().join(".loongclaw").join("rules");
    std::fs::create_dir_all(&rules_dir).expect("create rules dir");
    std::fs::write(rules_dir.join("broken.rules"), "not valid starlark(")
        .expect("write broken rule");

    let loongclaw = crate::config::LoongClawConfig::default();
    let config_path = root.path().join("loongclaw.toml");
    let runtime = ToolRuntimeConfig::from_loongclaw_config(&loongclaw, Some(&config_path));

    assert!(runtime.bash_exec.governance.load_error.is_some());
}
```

- [x] **Step 3: Run the red tests**

Run:

- `cargo test -p loongclaw-app tool_config_parses_bash_rules_dir_override --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_tool_config_defaults_to_no_explicit_rules_dir --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_uses_default_workspace_rules_dir_when_unset --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app tool_runtime_config_projects_bash_rules_dir_override --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_governance_runtime_treats_missing_rules_dir_as_empty_rule_set --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_governance_runtime_preserves_rule_load_error_for_broken_rule_file --lib -- --exact --nocapture`

Expected:

- FAIL because `BashToolConfig` has no `rules_dir`
- FAIL because `ToolRuntimeConfig` has no typed Bash governance runtime state
- FAIL because rule-directory resolution and rule-load states do not exist yet

- [x] **Step 4: Add dependencies, minimal rule types, and runtime scaffolding**

In `crates/app/Cargo.toml`, add the parser and rule-engine dependencies needed by the new modules:

- `tree-sitter`
- `tree-sitter-bash`
- `starlark`

Use crate-local dependency declarations unless promoting them to `[workspace.dependencies]` clearly improves consistency.

In `crates/app/src/config/tools.rs`:

- extend `BashToolConfig` to include `rules_dir: Option<String>`
- add a helper to resolve the effective rules directory from `config_path`
- keep `rules_dir = None` as the config default so runtime resolution can supply `~/.loongclaw/rules`

In `crates/app/src/tools/bash_rules.rs`, create the minimum bridge types needed for the runtime slice to compile:

- `PrefixRuleDecision`
- `CompiledPrefixRule`
- `load_rules_from_dir(...)`

Task-1 behavior for `load_rules_from_dir(...)` is intentionally minimal:

- return an empty rule set when the directory is missing
- if one or more `*.rules` files exist, return a fail-closed error until Task 2 adds the real Starlark loader

This keeps the Task-1 slice buildable while still preserving the fail-closed runtime behavior required for broken or unsupported rule files.

In `crates/app/src/tools/runtime_config.rs`, add:

```rust
#[derive(Debug, Clone)]
pub struct BashGovernanceRuntimePolicy {
    pub rules_dir: PathBuf,
    pub rules: Vec<crate::tools::bash_rules::CompiledPrefixRule>,
    pub load_error: Option<String>,
}
```

and extend `BashExecRuntimePolicy` with:

```rust
pub governance: BashGovernanceRuntimePolicy,
```

Build `governance` inside `ToolRuntimeConfig::from_loongclaw_config(...)` and `from_env()` by:

- resolving the rules directory
- calling `bash_rules::load_rules_from_dir(...)`
- translating `shell_allow` / `shell_deny` into `CompiledPrefixRule` values
- preserving any file-read / parse / compile error in `load_error`

For Task 1, that compatibility translation may stay inline in `runtime_config.rs` if that keeps the slice self-contained. Task 2 may later extract it behind `compile_compatibility_rules(...)` once the full rule-compiler module exists.

Do **not** change `ToolRuntimeConfig::from_loongclaw_config(...)` to return `Result`; preserve the existing constructor shape and carry fail-closed state in runtime policy.

In `crates/app/src/tools/bash.rs`, `crates/app/src/tools/catalog.rs`, and `crates/app/src/tools/mod.rs`:

- update every direct `BashExecRuntimePolicy { ... }` constructor to include the new `governance` field, preferably via a small helper or `..Default::default()` where that keeps the intermediate task slice concise and compiling

- [x] **Step 5: Run the green tests**

Run the six targeted tests from Step 3 again.

Expected: PASS

- [x] **Step 6: Run CI-parity checks and commit the scaffolding slice**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Then:

- `git add crates/app/Cargo.toml crates/app/src/tools/bash_rules.rs crates/app/src/config/tools.rs crates/app/src/tools/runtime_config.rs crates/app/src/tools/bash.rs crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): add bash governance runtime scaffolding"`

### Task 2: Implement rule loading and compatibility-rule compilation

**Files:**
- Modify: `crates/app/src/tools/bash_rules.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/bash_rules.rs`

- [x] **Step 1: Expand `bash_rules.rs` with red tests for compatibility translation**

Add tests to `crates/app/src/tools/bash_rules.rs`:

```rust
#[test]
fn shell_allow_entries_translate_to_single_token_allow_prefix_rules() {
    let rules = compile_compatibility_rules(
        &std::collections::BTreeSet::from(["cargo".to_owned()]),
        &std::collections::BTreeSet::new(),
    )
    .expect("compat rules");

    assert!(rules.iter().any(|rule| {
        rule.decision == PrefixRuleDecision::Allow && rule.pattern == vec!["cargo".to_owned()]
    }));
}

#[test]
fn shell_deny_entries_translate_to_single_token_deny_prefix_rules() {
    let rules = compile_compatibility_rules(
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::from(["rm".to_owned()]),
    )
    .expect("compat rules");

    assert!(rules.iter().any(|rule| {
        rule.decision == PrefixRuleDecision::Deny && rule.pattern == vec!["rm".to_owned()]
    }));
}
```

- [x] **Step 2: Write red tests for rule-file ordering and duplicates**

Add tests:

```rust
#[test]
fn rules_dir_loads_rule_files_in_stable_lexical_order() {
    let root = tempfile::tempdir().expect("tempdir");
    let rules_dir = root.path().join(".loongclaw").join("rules");
    std::fs::create_dir_all(&rules_dir).expect("create rules dir");
    std::fs::write(
        rules_dir.join("10-second.rules"),
        "prefix_rule(pattern=[\"cargo\",\"test\"], decision=\"allow\")\n",
    )
    .expect("write rule");
    std::fs::write(
        rules_dir.join("01-first.rules"),
        "prefix_rule(pattern=[\"cargo\",\"publish\"], decision=\"deny\")\n",
    )
    .expect("write rule");

    let loaded = load_rules_from_dir(&rules_dir).expect("load rules");
    assert_eq!(loaded[0].pattern, vec!["cargo".to_owned(), "publish".to_owned()]);
    assert_eq!(loaded[1].pattern, vec!["cargo".to_owned(), "test".to_owned()]);
}

#[test]
fn same_decision_duplicate_rules_can_be_normalized_away() {
    let compiled = compile_rules(vec![
        PrefixRuleSpec::allow(vec!["cargo".to_owned(), "test".to_owned()]),
        PrefixRuleSpec::allow(vec!["cargo".to_owned(), "test".to_owned()]),
    ])
    .expect("compile rules");

    assert_eq!(compiled.len(), 1);
}
```

- [x] **Step 3: Write red tests for rule-source precedence**

Add tests:

```rust
#[test]
fn deny_precedence_holds_even_when_allow_and_deny_come_from_different_sources() {
    let explicit = vec![PrefixRuleSpec::allow(vec!["cargo".to_owned(), "publish".to_owned()])];
    let compat = compile_compatibility_rules(
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::from(["cargo".to_owned()]),
    )
    .expect("compat rules");

    let compiled = merge_rule_sources(explicit, compat).expect("merge");
    let decision = evaluate_prefix_rules(
        &compiled,
        &["cargo".to_owned(), "publish".to_owned()],
    );

    assert_eq!(decision, Some(PrefixRuleDecision::Deny));
}
```

- [x] **Step 4: Run the red tests**

Run:

- `cargo test -p loongclaw-app shell_allow_entries_translate_to_single_token_allow_prefix_rules --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app shell_deny_entries_translate_to_single_token_deny_prefix_rules --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app rules_dir_loads_rule_files_in_stable_lexical_order --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app same_decision_duplicate_rules_can_be_normalized_away --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app deny_precedence_holds_even_when_allow_and_deny_come_from_different_sources --lib -- --exact --nocapture`

Expected: FAIL because the rule-loader module, compiled rule types, and Starlark-backed `prefix_rule(...)` loading do not exist yet.

- [x] **Step 5: Implement the rule compiler**

In `crates/app/src/tools/bash_rules.rs`, add:

- `PrefixRuleDecision`
- `CompiledPrefixRule`
- `PrefixRuleSpec`
- `compile_compatibility_rules(...)`
- `load_rules_from_dir(...)`
- `merge_rule_sources(...)`
- `evaluate_prefix_rules(...)`

Implementation requirements:

- replace the Task-1 fail-closed `load_rules_from_dir(...)` stub with the real Starlark-backed loader
- load only `*.rules` files
- sort file names lexically before compilation
- expose only `prefix_rule(...)` in the Starlark environment
- require `pattern` to be a non-empty string list
- allow only `decision = "allow"` or `decision = "deny"`
- normalize same-decision duplicates away during compilation
- preserve enough rule metadata to explain which rule matched later

Register the new module in `crates/app/src/tools/mod.rs`.

- [x] **Step 6: Run the green tests**

Run the five targeted tests from Step 4 again.

Expected: PASS

- [x] **Step 7: Run CI-parity checks and commit the rule-loader slice**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Then:

- `git add crates/app/src/tools/bash_rules.rs crates/app/src/tools/mod.rs crates/app/Cargo.toml`
- `git commit -m "feat(app): add bash prefix rule loading"`

### Task 3: Implement AST parsing and minimal-command-unit extraction

**Files:**
- Create: `crates/app/src/tools/bash_ast.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/bash_ast.rs`

- [x] **Step 1: Write red tests for supported outer splitting**

Create `crates/app/src/tools/bash_ast.rs` with tests first:

```rust
#[test]
fn splits_semicolon_lists_into_two_minimal_units() {
    let analysis = analyze_bash_command("cargo fmt ; cargo test").expect("analysis");
    assert_eq!(analysis.units.len(), 2);
}

#[test]
fn splits_and_and_or_lists_into_potentially_executable_units() {
    let analysis = analyze_bash_command("cd foo && cargo test || cargo test -- --nocapture")
        .expect("analysis");

    assert_eq!(analysis.units.len(), 3);
}
```

- [x] **Step 2: Write red tests for unsupported-unit classification**

Add tests:

```rust
#[test]
fn env_prefix_assignment_unit_is_classified_as_default_only() {
    let analysis = analyze_bash_command("FOO=1 cargo test").expect("analysis");
    assert_eq!(analysis.units[0].classification, UnitClassification::Unsupported(
        UnsupportedStructureKind::EnvPrefixAssignment
    ));
}

#[test]
fn pipeline_unit_is_classified_as_default_only() {
    let analysis = analyze_bash_command("cargo test | tee out.txt").expect("analysis");
    assert_eq!(analysis.units[0].classification, UnitClassification::Unsupported(
        UnsupportedStructureKind::Pipeline
    ));
}
```

- [x] **Step 3: Write red tests for parse errors**

Add tests:

```rust
#[test]
fn parse_error_marks_whole_command_unreliable() {
    let analysis = analyze_bash_command("if then").expect("analysis");
    assert!(analysis.parse_unreliable);
}

#[test]
fn command_substitution_unit_is_not_downgraded_to_plain_command() {
    let analysis = analyze_bash_command("echo $(git rev-parse HEAD)").expect("analysis");
    assert_eq!(analysis.units[0].classification, UnitClassification::Unsupported(
        UnsupportedStructureKind::CommandSubstitution
    ));
}
```

- [x] **Step 4: Run the red tests**

Run:

- `cargo test -p loongclaw-app splits_semicolon_lists_into_two_minimal_units --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app splits_and_and_or_lists_into_potentially_executable_units --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app env_prefix_assignment_unit_is_classified_as_default_only --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app pipeline_unit_is_classified_as_default_only --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app parse_error_marks_whole_command_unreliable --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app command_substitution_unit_is_not_downgraded_to_plain_command --lib -- --exact --nocapture`

Expected: FAIL because `tree-sitter-bash` parsing and unit classification do not exist yet.

- [x] **Step 5: Implement the parser and unit extractor**

In `crates/app/src/tools/bash_ast.rs`, add:

- a parser wrapper around `tree-sitter-bash`
- parse reliability detection using `has_error` / error / missing-node checks
- a top-level analysis type such as `BashCommandAnalysis`
- a `MinimalCommandUnit` type
- `UnitClassification` with:
  - `GovernablePlainCommand { argv: Vec<String> }`
  - `Unsupported(UnsupportedStructureKind)`
- splitting support for:
  - `;`
  - newline list forms
  - `&&`
  - `||`

Implementation rules:

- only plain simple-command units may produce governable argv output
- unsupported structures produce unit-level unsupported classifications
- parse unreliability marks the whole command for fallback
- preserve enough unit ordering / operator context for later conservative governance of `&&` and `||`
- do not downgrade unsupported shell structures into plain-command argv

- [x] **Step 6: Run the green tests**

Run the six targeted tests from Step 4 again.

Expected: PASS

- [x] **Step 7: Run CI-parity checks and commit the AST slice**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Then:

- `git add crates/app/src/tools/bash_ast.rs crates/app/src/tools/mod.rs crates/app/Cargo.toml`
- `git commit -m "feat(app): add bash ast unit extraction"`

### Task 4: Implement decision aggregation and wire it into `bash.exec`

**Files:**
- Create: `crates/app/src/tools/bash_governance.rs`
- Modify: `crates/app/src/tools/bash.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/bash_governance.rs`
- Test: `crates/app/src/tools/mod.rs`

- [x] **Step 1: Write red tests for whole-command aggregation**

Create `crates/app/src/tools/bash_governance.rs` with tests first:

```rust
#[test]
fn whole_command_allows_when_plain_command_matches_allow_prefix_rule() {
    let outcome = evaluate_bash_governance_for_test(
        "printf ok",
        PrefixRuleFixture::allow(["printf", "ok"]),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Deny,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Allow);
}

#[test]
fn unmatched_plain_command_uses_default_mode() {
    let outcome = evaluate_bash_governance_for_test(
        "printf ok",
        PrefixRuleFixture::none(),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Deny,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
}

#[test]
fn whole_command_denies_when_any_unit_denies() {
    let outcome = evaluate_bash_governance_for_test(
        "cargo publish && cargo test",
        PrefixRuleFixture::deny(["cargo", "publish"]),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Allow,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
}

#[test]
fn mixed_allow_and_default_resolves_through_default_mode() {
    let outcome = evaluate_bash_governance_for_test(
        "git status && cargo test | tee out.txt",
        PrefixRuleFixture::allow(["git", "status"]),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Deny,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
}

#[test]
fn or_list_denies_when_rhs_branch_matches_deny_rule() {
    let outcome = evaluate_bash_governance_for_test(
        "printf ok || printf blocked",
        PrefixRuleFixture::rules([
            PrefixRuleFixture::allow(["printf", "ok"]),
            PrefixRuleFixture::deny(["printf", "blocked"]),
        ]),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Allow,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Deny);
    assert_eq!(outcome.unit_outcomes.len(), 2);
}

#[test]
fn parse_unreliable_outcome_uses_default_allow_when_configured() {
    let outcome = evaluate_bash_governance_for_test(
        "if then",
        PrefixRuleFixture::none(),
        crate::tools::shell_policy_ext::ShellPolicyDefault::Allow,
    );

    assert_eq!(outcome.final_decision, FinalGovernanceDecision::Allow);
}
```

- [x] **Step 2: Write red tests for runtime load-error fail-closed behavior**

Add tests in `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_fails_closed_when_rule_loading_failed() {
    let mut config = test_tool_runtime_config(std::env::temp_dir());
    config.bash_exec.governance.load_error = Some("broken rules".to_owned());

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "cargo test"}),
        },
        &config,
    )
    .expect_err("broken rules should fail closed");

    assert!(error.contains("broken rules"));
}
```

- [x] **Step 3: Write red tests for `bash.exec` allow / deny / default behavior**

Add tests in `crates/app/src/tools/mod.rs`:

```rust
#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_allows_plain_command_when_prefix_rule_allows() {
    let root = unique_tool_temp_dir("loongclaw-bash-governance-allow");
    std::fs::create_dir_all(root.join(".loongclaw").join("rules")).expect("rules dir");
    std::fs::write(
        root.join(".loongclaw").join("rules").join("allow.rules"),
        "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    config.bash_exec = configured_test_bash_runtime_with_rules(&root);

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok"}),
        },
        &config,
    )
    .expect("allow rule should permit execution");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"], "ok");
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_denies_plain_command_when_prefix_rule_denies() {
    let root = unique_tool_temp_dir("loongclaw-bash-governance-deny");
    std::fs::create_dir_all(root.join(".loongclaw").join("rules")).expect("rules dir");
    std::fs::write(
        root.join(".loongclaw").join("rules").join("deny.rules"),
        "prefix_rule(pattern=[\"cargo\",\"publish\"], decision=\"deny\")\n",
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    config.bash_exec = configured_test_bash_runtime_with_rules(&root);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "cargo publish"}),
        },
        &config,
    )
    .expect_err("deny rule should block execution");

    assert!(error.contains("policy_denied"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_denies_or_list_when_rhs_branch_matches_deny_rule() {
    let root = unique_tool_temp_dir("loongclaw-bash-governance-or-deny");
    std::fs::create_dir_all(root.join(".loongclaw").join("rules")).expect("rules dir");
    std::fs::write(
        root.join(".loongclaw").join("rules").join("rules.rules"),
        concat!(
            "prefix_rule(pattern=[\"printf\",\"ok\"], decision=\"allow\")\n",
            "prefix_rule(pattern=[\"printf\",\"blocked\"], decision=\"deny\")\n",
        ),
    )
    .expect("rule file");

    let mut config = test_tool_runtime_config(root.clone());
    config.bash_exec = configured_test_bash_runtime_with_rules(&root);

    let error = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "printf ok || printf blocked"}),
        },
        &config,
    )
    .expect_err("conservative || governance should deny the rhs branch");

    assert!(error.contains("policy_denied"));
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_allows_parse_unreliable_command_when_shell_default_mode_is_allow() {
    let root = unique_tool_temp_dir("loongclaw-bash-governance-default-allow");
    let mut config = test_tool_runtime_config(root.clone());
    config.shell_default_mode = runtime_config::ShellPolicyDefault::Allow;
    config.bash_exec = configured_test_bash_runtime_with_rules(&root);

    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "bash.exec".to_owned(),
            payload: json!({"command": "if then"}),
        },
        &config,
    )
    .expect("default-allow should permit parse-unreliable input to reach bash");

    assert_eq!(outcome.status, "failed");
}

#[cfg(feature = "tool-shell")]
#[test]
fn bash_exec_keeps_shell_exec_unchanged() {
    let config = test_tool_runtime_config(std::env::temp_dir());
    let outcome = execute_tool_core_with_config(
        ToolCoreRequest {
            tool_name: "shell.exec".to_owned(),
            payload: json!({"command": "echo", "args": ["hi"]}),
        },
        &config,
    )
    .expect("shell.exec should remain runnable");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["stdout"], "hi");
}
```

- [x] **Step 4: Run the red tests**

Run:

- `cargo test -p loongclaw-app whole_command_allows_when_plain_command_matches_allow_prefix_rule --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app unmatched_plain_command_uses_default_mode --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app whole_command_denies_when_any_unit_denies --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app mixed_allow_and_default_resolves_through_default_mode --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app or_list_denies_when_rhs_branch_matches_deny_rule --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app parse_unreliable_outcome_uses_default_allow_when_configured --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_fails_closed_when_rule_loading_failed --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_allows_plain_command_when_prefix_rule_allows --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_denies_plain_command_when_prefix_rule_denies --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_denies_or_list_when_rhs_branch_matches_deny_rule --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_allows_parse_unreliable_command_when_shell_default_mode_is_allow --lib -- --exact --nocapture`
- `cargo test -p loongclaw-app bash_exec_keeps_shell_exec_unchanged --lib -- --exact --nocapture`

Expected: FAIL because the evaluator and `bash.exec` pre-execution governance hook do not exist yet.

- [x] **Step 5: Implement aggregation and executor wiring**

In `crates/app/src/tools/bash_governance.rs`, add:

- `UnitDecisionSource`
- `FinalGovernanceDecision`
- `BashGovernanceOutcome`
- `evaluate_bash_command(...)`

Implementation rules:

- explicit deny on any unit denies the whole command
- all-allow units allow the whole command
- any remaining `Default` unit resolves through `tools.shell_default_mode`
- include structured unit-level and final-decision metadata in the returned outcome

In `crates/app/src/tools/bash.rs`:

- before launching Bash, check `config.bash_exec.governance.load_error`
- return a fail-closed error if `load_error` is present
- otherwise evaluate governance on `payload.command`
- on deny, return `policy_denied: ...` with enough detail to distinguish:
  - explicit deny
  - unsupported-structure default deny
  - unmatched-prefix default deny
  - parse-unreliable default deny
- on allow, continue with the existing process execution path unchanged

- [x] **Step 6: Run the green tests**

Run the twelve targeted tests from Step 4 again.

Expected: PASS

- [x] **Step 7: Run CI-parity checks and commit the integration slice**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Then:

- `git add crates/app/src/tools/bash_governance.rs crates/app/src/tools/bash.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): govern bash.exec with ast prefix rules"`

### Task 5: Final regression pass and review convergence

**Files:**
- Modify: `docs/plans/2026-03-29-bash-governance-ast-prefix-rule-implementation-plan.md`
- Verify: `crates/app`

- [x] **Step 1: Run focused bash-governance regression tests**

Run:

- `cargo test -p loongclaw-app bash_governance --lib -- --nocapture`
- `cargo test -p loongclaw-app bash_exec_ --lib -- --nocapture`
- `cargo test -p loongclaw-app shell_ --lib -- --nocapture`

Expected:

- PASS for new Bash governance coverage
- PASS for existing `bash.exec` behavior that remains in scope
- PASS for adjacent `shell.exec` regressions

- [x] **Step 2: Run full CI-parity verification**

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`

Expected: PASS

- [x] **Step 3: Run ordinary correctness review to convergence**

Review the implementation against the spec with the chosen correctness mode:

- review mode: `auto-fix`
- fix any scope-in high or medium correctness / regression findings before moving on
- repeat until no high or medium correctness findings remain

- [x] **Step 4: Run one final style / simplicity / maintainability review**

Because the chosen style-review mode is `single-pass`:

- run one style / simplicity / maintainability review after correctness converges
- fix any scope-in high or medium style findings found in that pass
- do not repeat style review beyond that one required pass unless new correctness work reopens it

- [ ] **Step 5: Commit the final slice**

Run:

- `git add crates/app/Cargo.toml crates/app/src/config/tools.rs crates/app/src/tools/runtime_config.rs crates/app/src/tools/bash_rules.rs crates/app/src/tools/bash_ast.rs crates/app/src/tools/bash_governance.rs crates/app/src/tools/bash.rs crates/app/src/tools/mod.rs`
- `git commit -m "feat(app): add bash ast governance"`
