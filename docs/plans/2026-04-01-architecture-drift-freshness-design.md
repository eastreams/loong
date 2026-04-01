# Architecture Drift Freshness Determinism Design

## Goal

Make architecture drift freshness checks deterministic across output paths so CI and local
regeneration agree on whether a tracked monthly report is fresh.

## Current Repo Facts

- `scripts/generate_architecture_drift_report.sh`
  - derives the previous-month baseline from `dirname "$OUTPUT_PATH"` when
    `LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT` is unset
  - therefore treats the generated report path as the source of truth for baseline discovery
- `scripts/check_architecture_drift_freshness.sh`
  - regenerates the monthly report into `mktemp`
  - compares the tracked report against the temporary regenerated output after stripping the
    generated timestamp line
- monthly tracked reports live under `docs/releases/architecture-drift-YYYY-MM.md`
- existing script tests cover:
  - fresh tracked report succeeds
  - stale tracked report fails
  - untracked report path fails
  - no-baseline and explicit-baseline generation behavior
- existing tests do not cover the case where the tracked report and the regenerated temp file live
  in different directories but should still share the same previous-month baseline

## Root Cause

Freshness verification regenerates into a temp file, but report generation derives the baseline
from the temp file directory rather than from the tracked report month and tracked release report
location.

That makes baseline resolution path-sensitive:

1. tracked report path: `docs/releases/architecture-drift-2026-04.md`
2. freshness regeneration path: `/var/folders/.../tmp.xyz`
3. generator looks for baseline next to `/var/folders/.../tmp.xyz`
4. the real tracked baseline lives in `docs/releases/architecture-drift-2026-03.md`
5. the regenerated report records `Baseline report: none` instead of the tracked March report
6. freshness check reports drift even when the tracked report was generated correctly

This is a correctness bug in the governance seam, not a release-artifact content bug.

## Constraints

- keep the current CLI contract intact:
  - `scripts/generate_architecture_drift_report.sh [output_path]`
  - `scripts/check_architecture_drift_freshness.sh [report_path]`
- preserve explicit override behavior through `LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT`
- avoid hardcoding a single docs path into the script body when the caller already provides the
  tracked report path
- keep the fix small enough to remain a clean independent follow-up PR

## Options Considered

### Option 1: Teach the freshness script to regenerate directly into the tracked path directory

This would avoid the temp-path mismatch by generating a comparison artifact next to the tracked
report.

Why not:

- it couples the freshness script to output placement tricks instead of fixing the underlying
  baseline-resolution rule
- it keeps generation behavior path-sensitive for any other caller that uses a temp or alternate
  output path

### Option 2: Pass an explicit baseline override from freshness to generator

The freshness script could compute the previous tracked report path and export
`LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT`.

Why not:

- it duplicates baseline-resolution logic across two scripts
- it fixes only one caller and leaves other temp-output callers with the same hazard
- it increases maintenance burden for future report-call sites

### Option 3: Let generation accept an optional stable baseline directory and make freshness pass
the tracked report directory

This centralizes baseline resolution in generation while allowing callers to declare the canonical
release-report directory when output goes elsewhere.

Why this is the recommended option:

- fixes the root cause where it originates
- keeps the generator authoritative for baseline selection
- avoids hardcoding a single repository path into freshness logic
- preserves explicit baseline override precedence
- allows future callers to generate to temp paths without changing the month-to-month baseline
  semantics

## Recommended Design

Add one optional environment override to
`scripts/generate_architecture_drift_report.sh`:

- `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR`

Resolution rules:

1. if `LOONGCLAW_ARCH_DRIFT_BASELINE_REPORT` is set, use it exactly
2. otherwise, if `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` is set, derive the previous-month baseline
   inside that directory
3. otherwise, keep the current fallback of deriving the previous-month baseline from
   `dirname "$OUTPUT_PATH"`

Then update `scripts/check_architecture_drift_freshness.sh` to:

1. compute the directory of the tracked report path
2. export `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` to that tracked directory when regenerating into the
   temp file
3. leave timestamp normalization and tracked-file validation unchanged

## Why This Is The Smallest Correct Fix

- it changes one responsibility: how baseline lookup decides its directory when output path is not
  canonical
- it does not alter hotspot parsing, boundary checks, or report formatting
- it does not change tracked report contents when generation already happens under
  `docs/releases/`
- it creates one reusable seam instead of hiding a special case inside the freshness script

## Testing Strategy

Add regression coverage in `scripts/test_check_architecture_drift_freshness.sh`:

- seed a tracked March report in `docs/releases/`
- generate a tracked April report in `docs/releases/`
- verify freshness succeeds even though the checker regenerates into `mktemp`

Add generation coverage in `scripts/test_generate_architecture_drift_report.sh`:

- set `LOONGCLAW_ARCH_DRIFT_BASELINE_DIR` to a directory containing the previous-month report
- generate the new report into a different temp directory
- assert the emitted baseline label points at the stable baseline file, not `none`

## Scope Boundary

This PR will not:

- redesign the architecture drift report format
- change CI workflow structure
- update historical monthly report content
- solve broader release-doc automation beyond deterministic freshness comparison
