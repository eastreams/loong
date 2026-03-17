# LoongClaw Onboard Orchestrated Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add onboarding-driven legacy claw discovery, multi-source planning, safe profile-lane merge, and backup/rollback orchestration without breaking LoongClaw-native runtime ownership.

**Architecture:** Keep the existing single-source importer as the atomic building block. Add an app-layer orchestration core for discovery, scoring, plan-many, merge, and rollback; let daemon onboarding and spec hot actions call that shared core instead of reimplementing migration logic.

**Tech Stack:** Rust, `serde`, `serde_json`, existing `loongclaw-app` migration/config modules, daemon CLI onboarding flow, spec runtime tool extensions, cargo test, cargo fmt.

---

### Task 1: Add discovery and source scoring primitives

**Files:**
- Create: `crates/app/src/migration/orchestrator.rs`
- Modify: `crates/app/src/migration/mod.rs`
- Test: `crates/app/src/migration/orchestrator.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn discover_import_sources_returns_ranked_candidates_from_fixture_root() {
    let report = discover_import_sources(&fixture_root, DiscoveryOptions::default())
        .expect("discovery should succeed");
    assert_eq!(report.sources.len(), 2);
    assert_eq!(report.sources[0].source.as_id(), "openclaw");
    assert!(report.sources[0].confidence_score >= report.sources[1].confidence_score);
}

#[test]
fn discover_import_sources_ignores_empty_or_stock_only_noise_directories() {
    let report = discover_import_sources(&fixture_root, DiscoveryOptions::default())
        .expect("discovery should succeed");
    assert!(report.sources.iter().all(|item| item.path != noise_dir));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app discover_import_sources -- --nocapture`
Expected: FAIL because `discover_import_sources` and discovery report types do not exist yet.

**Step 3: Write minimal implementation**

Create deterministic orchestration types:

```rust
pub struct DiscoveryOptions {
    pub explicit_roots: Vec<PathBuf>,
}

pub struct DiscoveredImportSource {
    pub source: LegacyClawSource,
    pub path: PathBuf,
    pub confidence_score: u32,
    pub found_files: Vec<String>,
}

pub struct DiscoveryReport {
    pub sources: Vec<DiscoveredImportSource>,
}
```

Implement source discovery by scanning:

- explicit roots
- current directory
- nearby common claw directory names

Implement deterministic score inputs:

- explicit path bonus
- custom file count bonus
- structured identity bonus
- warning penalty

Sort `sources` descending by score and secondarily by canonical path.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app discover_import_sources -- --nocapture`
Expected: PASS with stable ordering.

**Step 5: Commit**

```bash
git add crates/app/src/migration/mod.rs crates/app/src/migration/orchestrator.rs
git commit -m "feat: add legacy claw discovery and scoring"
```

### Task 2: Add plan-many and primary recommendation

**Files:**
- Modify: `crates/app/src/migration/orchestrator.rs`
- Modify: `crates/app/src/tools/claw_migrate.rs`
- Test: `crates/app/src/migration/orchestrator.rs`
- Test: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn plan_import_sources_returns_summary_for_each_candidate() {
    let report = plan_import_sources(&fixture_sources).expect("plan-many should succeed");
    assert_eq!(report.plans.len(), 2);
    assert!(report.plans[0].profile_note_present);
}

#[test]
fn recommend_primary_source_prefers_richer_custom_source() {
    let recommendation = recommend_primary_source(&report).expect("primary source");
    assert_eq!(recommendation.source_id, "openclaw");
    assert!(!recommendation.reasons.is_empty());
}

#[test]
fn claw_migrate_supports_discover_and_plan_many_modes() {
    let outcome = execute_tool_core_with_config(request, &config).expect("tool should succeed");
    assert_eq!(outcome.payload["mode"], "discover");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app plan_import_sources -- --nocapture`
Run: `cargo test -p loongclaw-app recommend_primary_source -- --nocapture`
Run: `cargo test -p loongclaw-app claw_migrate_supports_discover_and_plan_many_modes -- --nocapture`
Expected: FAIL because orchestration modes and recommendation API do not exist.

**Step 3: Write minimal implementation**

Extend orchestration with:

```rust
pub struct ImportPlanSummary {
    pub source_id: String,
    pub input_path: PathBuf,
    pub confidence_score: u32,
    pub prompt_addendum_present: bool,
    pub profile_note_present: bool,
    pub warning_count: usize,
}

pub struct PrimarySourceRecommendation {
    pub source_id: String,
    pub reasons: Vec<String>,
}
```

Extend `claw.migrate` modes to accept:

- `discover`
- `plan_many`
- `recommend_primary`

Keep the existing `plan` and `apply` paths intact.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app plan_import_sources recommend_primary_source claw_migrate_supports_discover_and_plan_many_modes -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/migration/orchestrator.rs crates/app/src/tools/claw_migrate.rs crates/app/src/tools/mod.rs
git commit -m "feat: add migration planning and recommendation modes"
```

### Task 3: Integrate discovery and planning into onboard

**Files:**
- Modify: `crates/daemon/src/onboard_cli.rs`
- Modify: `crates/daemon/src/main.rs`
- Test: `crates/daemon/src/tests/onboard_cli.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn onboard_import_strategy_defaults_to_recommended_single_source() {
    let selection = resolve_onboard_import_strategy(&summary, false)
        .expect("strategy should resolve");
    assert!(matches!(selection.mode, OnboardImportMode::RecommendedSingleSource));
}

#[test]
fn onboard_import_summary_shows_safe_merge_as_secondary_option() {
    let summary = build_onboard_import_summary(&report);
    assert!(summary.contains("safe profile merge"));
}
```

If you expose new CLI flags, add parser coverage in daemon tests for:

- `migrate`
- `--import-strategy`
- `--skip-import`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon onboard_import_strategy -- --nocapture`
Expected: FAIL because onboard import orchestration types do not exist.

**Step 3: Write minimal implementation**

Refactor `run_onboard_cli(...)` so migration orchestration happens before the
provider/model/personality steps:

- discover sources
- if zero sources: continue normally
- if one source: offer import
- if multiple sources: recommend one source, allow manual single-source select,
  and expose safe profile merge as a secondary option

Introduce a small internal model:

```rust
enum OnboardImportMode {
    Skip,
    RecommendedSingleSource,
    SelectedSingleSource { source_id: String },
    SafeProfileMerge,
}
```

Do not implement merge here; just wire the onboarding selection flow.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon onboard_import_strategy -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/daemon/src/onboard_cli.rs crates/daemon/src/tests/onboard_cli.rs crates/daemon/src/main.rs
git commit -m "feat: integrate legacy source planning into onboarding"
```

### Task 4: Implement deterministic profile-lane merge and conflict reporting

**Files:**
- Create: `crates/app/src/migration/merge.rs`
- Modify: `crates/app/src/migration/mod.rs`
- Modify: `crates/app/src/migration/orchestrator.rs`
- Test: `crates/app/src/migration/merge.rs`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn merge_profile_entries_deduplicates_equivalent_entries() {
    let result = merge_profile_entries(entries).expect("merge should succeed");
    assert_eq!(result.kept_entries.len(), 1);
    assert_eq!(result.dropped_duplicates.len(), 1);
}

#[test]
fn merge_profile_entries_reports_same_slot_conflict() {
    let result = merge_profile_entries(entries).expect("merge should succeed");
    assert_eq!(result.unresolved_conflicts.len(), 1);
    assert!(!result.auto_apply_allowed);
}

#[test]
fn merge_profile_entries_never_changes_prompt_owner() {
    let result = merge_profile_entries(entries).expect("merge should succeed");
    assert_eq!(result.prompt_owner_source_id.as_deref(), Some("openclaw"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app merge_profile_entries -- --nocapture`
Expected: FAIL because merge engine does not exist.

**Step 3: Write minimal implementation**

Create normalized entry and conflict types:

```rust
pub struct ProfileMergeEntry {
    pub lane: ProfileEntryLane,
    pub canonical_text: String,
    pub source_id: String,
    pub source_confidence: u32,
    pub entry_confidence: u32,
    pub slot_key: Option<String>,
}

pub struct MergedProfilePlan {
    pub prompt_owner_source_id: Option<String>,
    pub merged_profile_note: String,
    pub unresolved_conflicts: Vec<ProfileMergeConflict>,
    pub auto_apply_allowed: bool,
}
```

Rules to encode:

- collapse exact duplicates
- prefer structured entries over free-form duplicates
- never merge prompt lane across sources
- unresolved same-slot conflicts block auto-apply

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app merge_profile_entries -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/migration/mod.rs crates/app/src/migration/orchestrator.rs crates/app/src/migration/merge.rs
git commit -m "feat: add deterministic profile merge and conflict reporting"
```

### Task 5: Expand `claw-migration` into orchestration actions

**Files:**
- Modify: `crates/spec/src/spec_runtime.rs`
- Modify: `crates/spec/src/kernel_bootstrap.rs`
- Modify: `crates/daemon/src/tests/spec_runtime.rs`
- Modify: `examples/spec/claw-import-hotplug.json`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[tokio::test]
async fn execute_spec_tool_extension_can_discover_multiple_sources() {
    let report = execute_spec(spec, true).await;
    assert_eq!(report.outcome["outcome"]["payload"]["action"], "discover");
    assert!(report.outcome["outcome"]["payload"]["sources"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn execute_spec_tool_extension_can_merge_profiles_without_merging_prompt_lane() {
    let report = execute_spec(spec, true).await;
    assert_eq!(report.outcome["outcome"]["payload"]["action"], "merge_profiles");
    assert_eq!(report.outcome["outcome"]["payload"]["result"]["prompt_owner_source_id"], "openclaw");
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon claw_migration -- --nocapture`
Expected: FAIL because only the simple `plan` wrapper exists today.

**Step 3: Write minimal implementation**

Extend the `claw-migration` extension to route explicit actions:

- `discover`
- `plan_many`
- `recommend_primary`
- `merge_profiles`
- `apply_selected`
- `rollback_last_apply`

Use app-layer orchestration functions directly. Do not reimplement scoring or
merge inside spec runtime.

Update the example spec file to demonstrate a multi-step discover/plan path.

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon claw_migration -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/spec/src/spec_runtime.rs crates/spec/src/kernel_bootstrap.rs crates/daemon/src/tests/spec_runtime.rs examples/spec/claw-import-hotplug.json
git commit -m "feat: expand claw-migration extension actions"
```

### Task 6: Add backup, manifest, rollback, and non-interactive gates

**Files:**
- Modify: `crates/app/src/migration/orchestrator.rs`
- Modify: `crates/app/src/tools/claw_migrate.rs`
- Modify: `crates/daemon/src/onboard_cli.rs`
- Test: `crates/app/src/migration/orchestrator.rs`
- Test: `crates/daemon/src/tests/onboard_cli.rs`
- Modify: `docs/plans/2026-03-11-loongclaw-migration-nativeization-implementation.md`

**Step 1: Write the failing tests**

Add tests for:

```rust
#[test]
fn apply_import_selection_writes_backup_and_manifest() {
    let result = apply_import_selection(selection).expect("apply should succeed");
    assert!(result.backup_path.exists());
    assert!(result.manifest_path.exists());
}

#[test]
fn rollback_last_import_restores_previous_config() {
    rollback_last_import(&target_config).expect("rollback should succeed");
    assert_eq!(fs::read_to_string(&target_config).unwrap(), original_body);
}

#[test]
fn non_interactive_onboard_blocks_multi_source_merge_without_explicit_opt_in() {
    let err = validate_non_interactive_import_strategy(strategy, false).expect_err("should block");
    assert!(err.contains("multi-source"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app apply_import_selection rollback_last_import -- --nocapture`
Run: `cargo test -p loongclaw-daemon non_interactive_onboard_blocks_multi_source_merge_without_explicit_opt_in -- --nocapture`
Expected: FAIL because backup/manifest/rollback and import-strategy safety gate do not exist.

**Step 3: Write minimal implementation**

Implement:

- manifest file under a deterministic migration state directory
- backup creation before apply
- rollback by latest successful manifest for target path
- non-interactive merge restrictions

Suggested manifest payload:

```json
{
  "session_id": "import-20260311-001",
  "selected_primary_source": "openclaw",
  "merged_sources": ["openclaw", "nanobot"],
  "prompt_owner_source": "openclaw",
  "output_path": "...",
  "backup_path": "...",
  "warnings": [],
  "unresolved_conflicts": 0
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app apply_import_selection rollback_last_import -- --nocapture`
Run: `cargo test -p loongclaw-daemon non_interactive_onboard_blocks_multi_source_merge_without_explicit_opt_in -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add crates/app/src/migration/orchestrator.rs crates/app/src/tools/claw_migrate.rs crates/daemon/src/onboard_cli.rs crates/daemon/src/tests/onboard_cli.rs docs/plans/2026-03-11-loongclaw-migration-nativeization-implementation.md
git commit -m "feat: add migration backup manifest and rollback"
```

### Task 7: Final verification and documentation pass

**Files:**
- Modify: `docs/plans/2026-03-11-loongclaw-onboard-orchestrated-migration-design.md`
- Modify: `docs/plans/2026-03-11-loongclaw-onboard-orchestrated-migration-implementation-plan.md`
- Modify: `README.md`
- Modify: `docs/product-specs/index.md`

**Step 1: Write the failing check list**

Create a written checklist in the implementation notes:

- onboarding detects zero, one, and many sources
- merge never blends prompt lane across sources
- rollback restores prior config
- non-interactive merge requires explicit opt-in

**Step 2: Run verification commands**

Run:

```bash
cargo fmt --all
cargo test -p loongclaw-app
cargo test -p loongclaw-spec
cargo test -p loongclaw-daemon
```

Expected: all commands succeed with zero failing tests.

**Step 3: Update docs with actual shipped commands and behavior**

Document:

- onboarding import behavior
- `claw.migrate` orchestration modes
- `claw-migration` action surface
- backup and rollback behavior

**Step 4: Commit**

```bash
git add README.md docs/product-specs/index.md docs/plans/2026-03-11-loongclaw-onboard-orchestrated-migration-design.md docs/plans/2026-03-11-loongclaw-onboard-orchestrated-migration-implementation-plan.md
git commit -m "docs: document orchestrated onboarding migration"
```
