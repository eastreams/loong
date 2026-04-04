# Architecture Drift Freshness Determinism Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make architecture drift freshness checks compare against a regeneration that uses the
same month-to-month baseline semantics as the tracked release report.

**Architecture:** Keep baseline resolution centralized in
`scripts/generate_architecture_drift_report.sh`, add one optional baseline-directory override, and
make the freshness checker pass the tracked report directory when regenerating to a temp file.
Cover the regression with shell tests before changing the scripts.

**Tech Stack:** Bash, git fixture repos, existing architecture budget shell test harness

---

## Task 1: Reproduce the temp-path baseline mismatch in tests

**Files:**
- Modify: `scripts/test_check_architecture_drift_freshness.sh`
- Test: `scripts/test_check_architecture_drift_freshness.sh`

**Step 1: Write the failing test**

Add a fixture test that:

- seeds `docs/releases/architecture-drift-2098-12.md`
- generates `docs/releases/architecture-drift-2099-01.md`
- tracks both reports in git
- runs `scripts/check_architecture_drift_freshness.sh` against the January report
- expects success because the checker should use the same December baseline even when it
  regenerates into a temp file

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_check_architecture_drift_freshness.sh`
Expected: FAIL in the new regression case because the freshness script regenerates with
`Baseline report: none`

**Step 3: Keep the failure output**

Confirm the failure is caused by the baseline mismatch, not by missing fixture data or an unrelated
diff.

## Task 2: Add generator coverage for stable baseline directory overrides

**Files:**
- Modify: `scripts/test_generate_architecture_drift_report.sh`
- Test: `scripts/test_generate_architecture_drift_report.sh`

**Step 1: Write the failing test**

Add a test that:

- creates a baseline report in one directory
- sets `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` to that directory
- generates the next-month report into another directory
- expects the generated report to record the baseline file path from the stable baseline directory

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_generate_architecture_drift_report.sh`
Expected: FAIL because the generator currently only derives the baseline from `dirname
"$OUTPUT_PATH"`

## Task 3: Implement the minimal baseline-directory override

**Files:**
- Modify: `scripts/generate_architecture_drift_report.sh`

**Step 1: Add the new override input**

Introduce `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` near the other report-generation inputs.

**Step 2: Keep resolution precedence explicit**

Make `resolve_baseline_path()` follow this order:

1. explicit baseline file override
2. explicit baseline directory override
3. output-directory fallback

**Step 3: Keep path handling simple**

Derive the previous-month filename once and join it against the chosen directory.

**Step 4: Run generator test to verify it passes**

Run: `bash scripts/test_generate_architecture_drift_report.sh`
Expected: PASS

## Task 4: Wire the freshness checker to the stable baseline directory

**Files:**
- Modify: `scripts/check_architecture_drift_freshness.sh`

**Step 1: Derive the tracked report directory**

Compute the directory from the tracked report path argument.

**Step 2: Export the stable baseline directory during regeneration**

Pass `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` when invoking
`scripts/generate_architecture_drift_report.sh "$TEMP_REPORT"`.

**Step 3: Run freshness test to verify it passes**

Run: `bash scripts/test_check_architecture_drift_freshness.sh`
Expected: PASS

## Task 5: Run syntax and governance verification

**Files:**
- Modify: none unless verification exposes follow-up fixes

**Step 1: Run shell syntax checks**

Run:

```bash
bash -n scripts/check_architecture_drift_freshness.sh
bash -n scripts/generate_architecture_drift_report.sh
bash -n scripts/test_check_architecture_drift_freshness.sh
bash -n scripts/test_generate_architecture_drift_report.sh
```

Expected: no syntax errors

**Step 2: Run targeted governance tests**

Run:

```bash
bash scripts/test_architecture_budget_scripts.sh
bash scripts/test_generate_architecture_drift_report.sh
bash scripts/test_check_architecture_drift_freshness.sh
```

Expected: all pass

**Step 3: Regenerate and verify the live tracked report**

Run:

```bash
REPORT_PATH="docs/releases/architecture-drift-$(date -u +%Y-%m).md"
scripts/generate_architecture_drift_report.sh "$REPORT_PATH"
scripts/check_architecture_drift_freshness.sh "$REPORT_PATH"
```

Expected: the tracked report remains fresh without requiring temp-path-specific content changes

### Task 6: Commit and prepare GitHub delivery

**Files:**
- Modify: `.github` artifacts only through `gh` commands, not repository files

**Step 1: Inspect isolated changes**

Run:

```bash
git status --short
git diff -- scripts/check_architecture_drift_freshness.sh scripts/generate_architecture_drift_report.sh scripts/test_check_architecture_drift_freshness.sh scripts/test_generate_architecture_drift_report.sh docs/plans/2026-04-01-architecture-drift-freshness-design.md docs/plans/2026-04-01-architecture-drift-freshness-implementation-plan.md
```

Expected: only the scoped script, test, and plan files changed

**Step 2: Commit with a task-scoped message**

Run:

```bash
git add scripts/check_architecture_drift_freshness.sh scripts/generate_architecture_drift_report.sh scripts/test_check_architecture_drift_freshness.sh scripts/test_generate_architecture_drift_report.sh docs/plans/2026-04-01-architecture-drift-freshness-design.md docs/plans/2026-04-01-architecture-drift-freshness-implementation-plan.md
git commit -m "Harden architecture drift freshness baseline resolution"
```

**Step 3: Create linked GitHub artifacts**

Use the issue template and PR template with English copy, a closing clause, and exact validation
commands.
