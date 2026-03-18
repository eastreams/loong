# Dev Main Release CI Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Align GitHub Actions with the repository's `dev -> main -> release` promotion model so `dev` receives continuous validation and branch protection can rely on a stable required check.

**Architecture:** Keep the current workflow split between CI, security, CodeQL, perf lint, and release publishing, but change the branch filters so the integration workflows follow the promotion chain instead of only `main`. Add a final aggregate `build` job in `ci.yml` so branch protection can depend on one stable check name even if individual jobs change later.

**Tech Stack:** GitHub Actions workflow YAML, shell validation, Rust workspace verification, Markdown planning docs

---

### Task 1: Align integration workflow trigger branches with the promotion chain

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/codeql.yml`
- Modify: `.github/workflows/security.yml`
- Modify: `.github/workflows/perf-lint.yml`
- Create: `.github/workflows/enforce-main-to-release.yml`
- Modify: `CONTRIBUTING.md`
- Modify: `docs/references/github-collaboration.md`

**Step 1: Update `ci.yml` branch filters**

Change the `on:` block so:
- `pull_request.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- `push.branches` explicitly includes `dev`, `main`, `release`, and `release/**`

Do not change the existing job bodies in this step.

**Step 2: Update `codeql.yml` branch filters**

Change the `on:` block so:
- `pull_request.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- `push.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- keep the weekly `schedule` unchanged

**Step 3: Update `security.yml` branch filters**

Change the `on:` block so:
- `pull_request.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- `push.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- keep the weekly `schedule` unchanged

**Step 4: Update `perf-lint.yml` branch filters**

Keep the current `paths` filter, but add matching branch filters so:
- `pull_request.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- `push.branches` explicitly includes `dev`, `main`, `release`, and `release/**`
- `push.paths` matches the existing path-scoped files so normal pushes do not trigger unrelated perf lint runs

This keeps the benchmark-lint workflow scoped to relevant files while making it available on `dev`.

**Step 5: Add release-lane promotion enforcement**

Create `.github/workflows/enforce-main-to-release.yml` mirroring the style of
`enforce-dev-to-main.yml`, but scoped to:
- `pull_request_target.branches`: `release`, `release/**`
- source branch requirement: `main`

The workflow should comment, close the PR, and fail the job when a release-lane PR comes from a
branch other than `main`.

**Step 6: Update contributor-facing collaboration docs**

Document the full `dev -> main -> release` promotion chain in:
- `CONTRIBUTING.md`
- `docs/references/github-collaboration.md`

Also document that the stable required status check is the aggregate `build` job from `ci.yml`.

**Step 7: Run targeted YAML syntax validation**

Run:

```bash
ruby -e 'require "yaml"; ARGV.each { |path| YAML.load_file(path) }' \
  .github/workflows/ci.yml \
  .github/workflows/codeql.yml \
  .github/workflows/security.yml \
  .github/workflows/perf-lint.yml \
  .github/workflows/enforce-main-to-release.yml
```

Expected: command exits `0` with no syntax errors.

### Task 2: Add a stable aggregate required check for branch protection

**Files:**
- Modify: `.github/workflows/ci.yml`

**Step 1: Add an aggregate `build` job**

Append a new job named `build` that:
- uses `needs` on `governance`, `rust-quality`, `rust-test-default`, `rust-test-all-features`, and `docs-build`
- runs on `ubuntu-latest`
- uses `if: ${{ always() }}` so the aggregate check still reports when an upstream job fails
- fails unless every dependency result is exactly `success`

Use an environment-driven shell step so the failure output makes it clear which upstream result blocked the aggregate check.

**Step 2: Re-run focused workflow validation**

Run:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml")'
```

Expected: command exits `0`.

**Step 3: Check the rendered diff for workflow hygiene**

Run:

```bash
git diff -- .github/workflows/ci.yml .github/workflows/codeql.yml .github/workflows/security.yml .github/workflows/perf-lint.yml
git diff --check
```

Expected: only the intended workflow trigger and aggregate-check changes appear, with no whitespace errors.

### Task 3: Verify repository baseline and post-change behavior evidence

**Files:**
- Modify: `docs/plans/2026-03-18-dev-main-release-ci-lifecycle-implementation-plan.md`

**Step 1: Run the repository baseline test suite once in the clean worktree**

Run:

```bash
cargo test --workspace --locked
```

Expected: if the baseline is green, record that evidence; if it fails, capture the failure and continue only with that pre-existing risk noted.

**Step 2: Re-run the validation commands after the workflow edits**

Run:

```bash
ruby -e 'require "yaml"; ARGV.each { |path| YAML.load_file(path) }' \
  .github/workflows/ci.yml \
  .github/workflows/codeql.yml \
  .github/workflows/security.yml \
  .github/workflows/perf-lint.yml \
  .github/workflows/enforce-main-to-release.yml
git diff --check
```

Expected: both commands exit `0`.

**Step 3: Summarize the required GitHub-side follow-up**

Record in the final handoff that branch protection or rulesets should require the aggregate `build` check rather than a nonexistent legacy name. Do not mutate GitHub rulesets from this task unless explicitly requested.
