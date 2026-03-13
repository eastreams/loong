# Architecture Drift Review Implementation

Goal: turn the architecture SLO and refactor-budget governance checklist items into executable,
tracked repository artifacts.

Status: Implemented and validated on the current task branch

## Delivered

1. Added shared hotspot budget definitions in `scripts/architecture_budget_lib.sh`.
2. Kept the live architecture gate green while reusing the shared hotspot scope.
3. Added `scripts/generate_architecture_drift_report.sh` to generate monthly markdown drift
   artifacts with embedded baseline markers.
4. Generated and tracked `docs/releases/architecture-drift-2026-03.md`.
5. Added an explicit release-doc `Refactor Budget` policy and enforcement.

## Minimal Production Change

Modified:

- `scripts/architecture_budget_lib.sh`
- `scripts/check_architecture_boundaries.sh`
- `scripts/generate_architecture_drift_report.sh`
- `scripts/test_generate_architecture_drift_report.sh`
- `scripts/check-docs.sh`
- `docs/releases/README.md`
- `docs/releases/TEMPLATE.md`
- `docs/releases/v0.1.0.md`
- `docs/releases/v0.1.1.md`
- `docs/releases/v0.1.2.md`
- `docs/releases/architecture-drift-2026-03.md`
- `docs/plans/2026-03-13-architecture-drift-review-design.md`
- `docs/plans/2026-03-13-architecture-drift-review.md`

## Validation

Commands completed after the change:

```bash
bash -n scripts/architecture_budget_lib.sh
bash -n scripts/check_architecture_boundaries.sh
bash -n scripts/generate_architecture_drift_report.sh
bash -n scripts/test_generate_architecture_drift_report.sh
bash scripts/test_generate_architecture_drift_report.sh
LOONGCLAW_ARCH_STRICT=true scripts/check_architecture_boundaries.sh
./scripts/generate_architecture_drift_report.sh docs/releases/architecture-drift-2026-03.md
scripts/bootstrap_release_local_artifacts.sh
LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh
git diff --check
```

Observed results:

- shared architecture budget library syntax: PASS
- architecture gate: PASS
- generator syntax and regression tests: PASS
- monthly drift artifact generation: PASS
- doc governance checks: PASS in strict mode after bootstrap
- `git diff --check`: PASS

## Outcome

The two remaining long-term governance items are now operational:

1. monthly architecture drift review produces a tracked artifact under `docs/releases/`
2. release docs now carry an explicit refactor-budget item backed by doc-governance checks
