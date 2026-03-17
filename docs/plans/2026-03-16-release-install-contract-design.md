# Release Install Contract Design

## Goal

Close the release-first install contract for LoongClaw so the shipped
installers, product docs, and tests all describe the same real behavior when
GitHub releases exist and when they do not.

## Current State

- `README.md` already exposes a release-first installer path and documents a
  source fallback when no GitHub release is published yet.
- `scripts/install.sh` and `scripts/install.ps1` already prefer GitHub Release
  assets and fail closed when `releases/latest` is missing.
- The Bash installer is not easily smoke-testable against local release
  fixtures because the download base is hard-coded to GitHub.
- There is no end-to-end smoke test for the actual `scripts/install.sh`
  entrypoint.
- Public GitHub release APIs currently report no published releases for
  `loongclaw-ai/loongclaw`, so users still land on the fallback path today.
- `docs/product-specs/installation.md` still looks like a draft because every
  acceptance box is unchecked.

## Problem

The repository currently has the right release-first shape but not the full
contract:

1. the installers are hard to verify end-to-end without talking to GitHub
2. the missing-release path is honest but not actionable enough
3. the public install docs and spec do not clearly communicate today's real
   first-run path
4. release-first support therefore exists more as infrastructure than as a
   reliably testable user contract

That is a user-experience gap for MVP, because install is the first moment where
the product either feels trustworthy or not.

## Chosen Slice

Implement a narrow install-contract slice instead of expanding into package
managers or heavier release tooling:

1. Add a local release-base override to both installers so they can be tested
   against fixture assets.
2. Add a real smoke test for `scripts/install.sh` that covers successful
   install, checksum failure, and missing-release guidance.
3. Improve missing-release guidance with exact next actions instead of a vague
   fallback hint.
4. Tighten README and installation spec wording so the quickstart stays honest
   before the first public release exists.

This gives the MVP a sturdier install story without introducing package-manager
distribution, auto-update logic, or broader release orchestration.

## Design

### 1. Testable installer inputs

Introduce a download-base override for installer fetches:

- `scripts/install.sh` reads `LOONGCLAW_INSTALL_RELEASE_BASE_URL` before falling
  back to `https://github.com/<repo>/releases`.
- `scripts/install.ps1` mirrors that override through the same environment
  variable.

The override is intentionally narrow: it only changes the base URL used for the
archive and checksum downloads, leaving release-tag resolution unchanged unless
the caller also pins `--version`.

### 2. Bash smoke coverage

Add `scripts/test_install_sh.sh` as an end-to-end shell smoke test for the
actual installer script:

- install from a local release fixture through the override base URL
- fail on a deliberately corrupted checksum file
- fail with exact source-install guidance when `releases/latest` is absent

This test operates entirely on temp directories and fixture assets, so it does
not require a published GitHub release.

### 3. Missing-release guidance

When the latest-release lookup fails, both installers should print an
immediately actionable next path:

- clone the repository from GitHub
- run the source installer from that checkout
- optionally continue straight into onboarding

The message should be copy-pastable and should not assume the user already knows
what a “repository checkout” means.

### 4. Product docs

The public install docs should explicitly say:

- the bootstrap installer is release-first when assets exist
- no public release is published in the repository today
- the supported immediate fallback is the source installer below
- `--onboard` is the fastest path into a first useful answer

This keeps the product honest without downgrading the release-first direction.

## Non-Goals

- Homebrew, winget, apt, or other package-manager distribution
- Auto-update / self-update
- Linux ARM64 packaging or broader target-matrix changes
- Retrofitting release docs or CI governance into a larger release audit

## Risks

- The shell smoke test only covers the Bash entrypoint. Mitigation: mirror the
  PowerShell behavior closely and document local verification limits if `pwsh`
  is unavailable.
- A new override could become user-facing surface area unintentionally.
  Mitigation: keep it undocumented and scoped to testability, not product docs.

## Acceptance Criteria

- `scripts/test_install_sh.sh` fails before implementation and passes after.
- `scripts/install.sh` supports a release-base override, installs successfully
  from a local fixture, and fails closed on checksum drift.
- Both installers print exact source-install next steps when no public release
  exists.
- `README.md` and `docs/product-specs/installation.md` match the shipped
  release-first-with-source-fallback behavior.
