# Linux Musl Release Contract Tasks

## Objective

Ship the approved Linux GNU plus musl release contract for `x86_64`, wire the
Bash installer to choose a compatible libc variant by default, and verify the
change against the repo's existing shell and release gates.

## Checklist

- [x] Inspect current installer, release helper, and release workflow behavior.
- [x] Confirm the Debian 12 failure mode and current public release contract.
- [x] Align the spec location and format with existing `docs/plans` documents.
- [x] Write `docs/plans/2026-03-20-linux-musl-release-contract-design.md`.
- [x] Perform a local review pass for contract gaps and scope drift.
- [x] Commit the approved design and ask for user review.
- [x] Post a concise implementation update to GitHub issue `#310`.
- [x] Write `docs/plans/2026-03-20-linux-musl-release-contract-implementation-plan.md`.
- [x] Add failing helper and installer coverage for libc-aware Linux behavior.
- [x] Implement shared release-helper metadata, installer selection, and release
      workflow updates for Linux `x86_64` GNU plus musl artifacts.
- [x] Update public install docs to describe auto-selection and manual override.
- [x] Run targeted shell regression checks and repo verification.
- [x] Clear the pre-existing `cargo deny` advisory gate so `task verify` can go
      green on this branch.
- [x] Clear the newly surfaced `cargo audit` advisories in `aws-lc-sys` so PR
      Security checks pass.
- [x] Address the remaining PR review threads around Linux libc detection,
      version comparison portability, GNU-only fallback safety, and workflow
      glibc floor reuse.

## Progress Notes

- 2026-03-20: Confirmed the current Linux release contract is GNU-only in
  `scripts/release_artifact_lib.sh`, `scripts/install.sh`, and
  `.github/workflows/release.yml`.
- 2026-03-20: Confirmed the Bash installer is the Linux path; `install.ps1`
  remains Windows-only, so the first musl slice stays in the Bash/shared helper
  contract.
- 2026-03-20: Confirmed the release workflow already enforces a Linux ARM64
  glibc floor through `scripts/check_glibc_floor.sh`, which can be extended for
  explicit GNU floor metadata instead of inventing a second mechanism.
- 2026-03-20: Wrote the design doc in `docs/plans` and tightened the contract
  around explicit GNU override behavior, glibc detection order, and shared
  helper ownership.
- 2026-03-20: Posted the agreed rollout direction to GitHub issue `#310` with a
  concise summary of the Debian 12 repro, dual-artifact contract, installer
  fallback rule, and first-pass `x86_64` scope.
- 2026-03-20: Wrote the implementation plan in `docs/plans` and executed it
  helper-first: add failing tests, implement shared libc metadata, then wire the
  installer selection logic and release workflow.
- 2026-03-20: Added release-helper coverage for Linux musl archive/checksum
  naming, supported libc variants, and GNU glibc floor metadata; the first run
  failed as expected before `release_supported_linux_libcs_for_arch` and related
  helpers were implemented.
- 2026-03-20: Added installer regression coverage for GNU preference on
  supported glibc, musl fallback on old or unreadable glibc, and explicit
  `gnu|musl` override behavior; the first run failed until the installer learned
  host glibc detection and target selection.
- 2026-03-20: Extended the release workflow to publish
  `x86_64-unknown-linux-musl`, install `musl-tools` for that target, and apply
  glibc floor checks only to GNU Linux targets.
- 2026-03-20: Updated `README.md` and `docs/product-specs/installation.md` so
  the public contract matches the shipped installer behavior.
- 2026-03-21: Cleared the repo-wide verification blocker with a narrow lockfile
  update from `rustls-webpki 0.103.9` to `0.103.10`, matching the
  `RUSTSEC-2026-0049` remediation guidance without widening the dependency
  surface beyond the affected crate.
- 2026-03-21: Reproduced the PR Security failure locally with `cargo audit`,
  which surfaced `RUSTSEC-2026-0044` and `RUSTSEC-2026-0048` through
  `aws-lc-sys 0.38.0` via `aws-lc-rs 1.16.1`.
- 2026-03-21: Cleared the Security gate with the narrow compatible lockfile
  update `aws-lc-rs 1.16.1 -> 1.16.2` and `aws-lc-sys 0.38.0 -> 0.39.0`,
  matching the advisory remediation without changing application code or
  release-contract behavior.
- 2026-03-21: Reviewed each open PR bot thread against the shipped shell path,
  reproduced the valid failures locally, and kept the fixes narrow: explicit
  unsupported-arch failure propagation in the release helper, musl-aware glibc
  detection, `sort -V` fallback coverage, GNU-only arch rejection when no musl
  artifact exists, and shared glibc floor lookup in the release workflow.
- 2026-03-21: After the review-follow-up push, GitHub Actions exposed one
  Linux-only test harness gap in the standalone installer regression: the copied
  installer still saw the host runner's real glibc via `getconf`/`ldd`. The
  fix was test-only and narrow: stub both commands to fail so the test actually
  exercises the intended "glibc unavailable" path on CI.

## Review / Results

- 2026-03-20: Local design review completed. The main gap was explicit override
  safety: the final contract requires the installer to fail early when `gnu` is
  forced on a host that does not meet the declared GNU glibc floor.
- 2026-03-20: Targeted verification passed:
  `bash scripts/test_release_artifact_lib.sh`,
  `bash scripts/test_install_sh.sh`,
  `bash scripts/test_check_glibc_floor.sh`, and `git diff --check`.
- 2026-03-20: `task verify` completed all relevant build/test checks for this
  change and failed only on the pre-existing unrelated `cargo deny` advisory
  `RUSTSEC-2026-0049` in `rustls-webpki 0.103.9`.
- 2026-03-20: Intentional first-pass scope remains Linux `x86_64`; `aarch64`
  musl support is left as a follow-up matrix extension.
- 2026-03-21: Follow-up verification passed after the lockfile bump:
  `cargo deny check advisories` and full `task verify` are green on this
  branch. Remaining `cargo deny` output is warning-only duplicate/license noise,
  not a failing advisory gate.
- 2026-03-21: Security follow-up verification passed:
  `cargo audit`, `cargo deny check advisories bans sources`, and full
  `task verify` are green after the AWS-LC lockfile update.
- 2026-03-21: Review follow-up verification passed:
  `bash scripts/test_release_artifact_lib.sh`,
  `bash scripts/test_install_sh.sh`,
  `bash scripts/test_check_glibc_floor.sh`,
  `git diff --check`, and full `task verify` are green after the review-thread
  fixes. The standalone copied-installer regression now intentionally fails on
  GNU-only `aarch64` when no compatible glibc can be detected, matching the
  reviewed contract instead of silently installing an unusable binary.
- 2026-03-21: CI parity follow-up passed:
  the exact governance regression-test bundle from `.github/workflows/ci.yml`
  now passes locally after stubbing `getconf` and `ldd` in the standalone
  Linux `aarch64` test, and `task verify` remains green.
