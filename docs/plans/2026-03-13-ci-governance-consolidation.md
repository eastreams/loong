# CI Governance Consolidation Implementation

Goal: make `ci.yml` the single primary PR gate and move mature governance checks into an explicit,
reviewable CI job.

Status: Implemented and validated

## Delivered

1. Consolidated the main gate into `.github/workflows/ci.yml`.
2. Removed redundant `.github/workflows/verify.yml`.
3. Added release-artifact bootstrap and strict release-doc governance helpers.
4. Added architecture drift report generation and regression coverage.
5. Added daemon stress summary regression coverage.
6. Kept the architecture boundary library compatible with the current `alpha-test` code layout.

## Production Files

- `.github/workflows/ci.yml`
- `.github/workflows/verify.yml` (removed)
- `scripts/architecture_budget_lib.sh`
- `scripts/check_architecture_boundaries.sh`
- `scripts/generate_architecture_drift_report.sh`
- `scripts/test_generate_architecture_drift_report.sh`
- `scripts/release_artifact_lib.sh`
- `scripts/bootstrap_release_local_artifacts.sh`
- `scripts/test_release_artifact_lib.sh`
- `scripts/test_bootstrap_release_local_artifacts.sh`
- `scripts/check-docs.sh`
- `scripts/stress_daemon_tests.sh`
- `scripts/test_stress_daemon_tests.sh`
- `docs/releases/README.md`
- `docs/releases/TEMPLATE.md`
- `docs/releases/v0.1.0.md`
- `docs/releases/v0.1.1.md`
- `docs/releases/v0.1.2.md`
- `docs/releases/architecture-drift-2026-03.md`
- `docs/plans/2026-03-13-ci-governance-consolidation-design.md`
- `docs/plans/2026-03-13-ci-governance-consolidation.md`

## Validation

Commands completed after the change:

```bash
bash -n scripts/architecture_budget_lib.sh
bash -n scripts/check_architecture_boundaries.sh
bash -n scripts/generate_architecture_drift_report.sh
bash -n scripts/test_generate_architecture_drift_report.sh
bash -n scripts/release_artifact_lib.sh
bash -n scripts/bootstrap_release_local_artifacts.sh
bash -n scripts/test_release_artifact_lib.sh
bash -n scripts/test_bootstrap_release_local_artifacts.sh
bash -n scripts/check-docs.sh
bash -n scripts/stress_daemon_tests.sh
bash -n scripts/test_stress_daemon_tests.sh
bash scripts/test_generate_architecture_drift_report.sh
bash scripts/test_release_artifact_lib.sh
bash scripts/test_bootstrap_release_local_artifacts.sh
bash scripts/test_stress_daemon_tests.sh
scripts/bootstrap_release_local_artifacts.sh
LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh
LOONGCLAW_ARCH_STRICT=true scripts/check_architecture_boundaries.sh
scripts/check_dep_graph.sh
git diff --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo doc --workspace --no-deps
```

Observed results:

- governance shell syntax checks: PASS
- governance regression scripts: PASS
- release-artifact bootstrap plus strict doc governance: PASS
- strict architecture and dependency graph contracts: PASS
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: PASS
- `cargo test --workspace --locked`: PASS
- `cargo test --workspace --all-features --locked`: PASS
- `cargo doc --workspace --no-deps`: PASS
