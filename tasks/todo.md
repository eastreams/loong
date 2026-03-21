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
