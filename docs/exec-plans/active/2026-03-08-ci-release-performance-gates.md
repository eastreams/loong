# CI Release Performance Gates Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement a fast PR gate plus full nightly/release performance benchmark model without materially slowing normal contributor loops.

**Architecture:** Keep correctness/security checks in existing CI and add a dedicated performance split: lightweight baseline-lint on PR, full benchmark on nightly/release preflight. Enforce publish blocking in `release.yml` so benchmark regressions cannot ship. Preserve observability by uploading benchmark reports as artifacts and documenting required status checks.

**Tech Stack:** GitHub Actions workflows, Bash scripts in `scripts/`, Cargo task runner (`Taskfile.yml`), benchmark matrix/baseline JSON, markdown docs.

---

### Task 1: Add PR Performance Lint Workflow (Fast, Blocking)

**Files:**
- Create: `.github/workflows/perf-lint.yml`
- Test: `.github/workflows/perf-lint.yml`

**Step 1: Write the failing test**

```bash
test -f .github/workflows/perf-lint.yml
```

**Step 2: Run test to verify it fails**

Run: `test -f .github/workflows/perf-lint.yml`  
Expected: non-zero exit code (file missing).

**Step 3: Write minimal implementation**

```yaml
name: perf-lint

on:
  pull_request:
    paths:
      - '.github/workflows/**'
      - 'scripts/lint_programmatic_pressure_baseline.sh'
      - 'examples/benchmarks/**'
      - 'crates/daemon/**'
      - 'crates/spec/**'
      - 'crates/kernel/**'
      - 'crates/app/**'
  push:
    branches: [main]

concurrency:
  group: perf-lint-${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

jobs:
  perf-lint:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: ./scripts/lint_programmatic_pressure_baseline.sh
```

**Step 4: Run test to verify it passes**

Run:

```bash
test -f .github/workflows/perf-lint.yml
rg -n 'name:\s*perf-lint|lint_programmatic_pressure_baseline.sh|concurrency:|permissions:' .github/workflows/perf-lint.yml
```

Expected: file exists and patterns are found.

**Step 5: Commit**

```bash
git add .github/workflows/perf-lint.yml
git commit -m "ci: add fast perf-lint workflow for PR gate"
```

### Task 2: Add Full Benchmark Workflow (Nightly + Manual)

**Files:**
- Create: `.github/workflows/perf-benchmark.yml`
- Test: `.github/workflows/perf-benchmark.yml`

**Step 1: Write the failing test**

```bash
test -f .github/workflows/perf-benchmark.yml
```

**Step 2: Run test to verify it fails**

Run: `test -f .github/workflows/perf-benchmark.yml`  
Expected: non-zero exit code (file missing).

**Step 3: Write minimal implementation**

```yaml
name: perf-benchmark

on:
  schedule:
    - cron: '0 6 * * *'
  workflow_dispatch:

concurrency:
  group: perf-benchmark-${{ github.ref }}
  cancel-in-progress: false

jobs:
  benchmark:
    runs-on: ubuntu-latest
    timeout-minutes: 45
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: ./scripts/benchmark_programmatic_pressure.sh
      - uses: actions/upload-artifact@v4
        with:
          name: programmatic-pressure-report-${{ github.run_id }}
          path: target/benchmarks/programmatic-pressure-report.json
          retention-days: 30
```

**Step 4: Run test to verify it passes**

Run:

```bash
test -f .github/workflows/perf-benchmark.yml
rg -n 'schedule:|workflow_dispatch:|benchmark_programmatic_pressure.sh|upload-artifact|retention-days' .github/workflows/perf-benchmark.yml
```

Expected: file exists and all required patterns are found.

**Step 5: Commit**

```bash
git add .github/workflows/perf-benchmark.yml
git commit -m "ci: add nightly full performance benchmark workflow"
```

### Task 3: Add Release Preflight Benchmark Gate

**Files:**
- Modify: `.github/workflows/release.yml`
- Test: `.github/workflows/release.yml`

**Step 1: Write the failing test**

```bash
rg -n 'benchmark_programmatic_pressure.sh|needs:.*perf-preflight|name:\s*perf-preflight' .github/workflows/release.yml
```

**Step 2: Run test to verify it fails**

Run: same command as Step 1  
Expected: no matches for one or more required patterns.

**Step 3: Write minimal implementation**

```yaml
jobs:
  perf-preflight:
    runs-on: ubuntu-latest
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: ./scripts/benchmark_programmatic_pressure.sh

  publish-release:
    needs: perf-preflight
```

Keep existing release creation step unchanged.

**Step 4: Run test to verify it passes**

Run:

```bash
rg -n 'name:\s*perf-preflight|benchmark_programmatic_pressure.sh|needs:\s*perf-preflight' .github/workflows/release.yml
```

Expected: all patterns found.

**Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "release: block publish on benchmark preflight"
```

### Task 4: Add Local Taskfile Entrypoints for Perf Lint and Perf Benchmark

**Files:**
- Modify: `Taskfile.yml`
- Test: `Taskfile.yml`

**Step 1: Write the failing test**

```bash
rg -n '^\s*perf:lint:|^\s*perf:benchmark:' Taskfile.yml
```

**Step 2: Run test to verify it fails**

Run: same command as Step 1  
Expected: one or both tasks missing.

**Step 3: Write minimal implementation**

```yaml
  perf:lint:
    desc: Lint performance benchmark baseline and schema coverage
    cmds:
      - ./scripts/lint_programmatic_pressure_baseline.sh

  perf:benchmark:
    desc: Run full performance benchmark gate
    cmds:
      - ./scripts/benchmark_programmatic_pressure.sh
```

**Step 4: Run test to verify it passes**

Run:

```bash
rg -n '^\s*perf:lint:|^\s*perf:benchmark:' Taskfile.yml
task perf:lint
```

Expected: tasks exist and `task perf:lint` exits 0.

**Step 5: Commit**

```bash
git add Taskfile.yml
git commit -m "build: add local perf lint and benchmark tasks"
```

### Task 5: Document Required Checks and Performance Gate Policy

**Files:**
- Create: `docs/references/ci-performance-gates.md`
- Modify: `docs/RELIABILITY.md`
- Modify: `docs/index.md`
- Test: docs grep + link check script

**Step 1: Write the failing test**

```bash
test -f docs/references/ci-performance-gates.md
```

**Step 2: Run test to verify it fails**

Run: `test -f docs/references/ci-performance-gates.md`  
Expected: non-zero exit code.

**Step 3: Write minimal implementation**

```markdown
# CI Performance Gates

## Required PR checks
- verify
- security
- perf-lint

## Nightly checks
- perf-benchmark

## Release gate
- perf-preflight must pass before publish-release.
```

Add links from `docs/index.md`; in `docs/RELIABILITY.md` clarify fast PR path vs full nightly/release benchmark.

**Step 4: Run test to verify it passes**

Run:

```bash
test -f docs/references/ci-performance-gates.md
rg -n 'perf-lint|perf-benchmark|perf-preflight' docs/references/ci-performance-gates.md docs/RELIABILITY.md docs/index.md
bash scripts/check-docs.sh
```

Expected: file exists, policy strings found, and docs check passes.

**Step 5: Commit**

```bash
git add docs/references/ci-performance-gates.md docs/RELIABILITY.md docs/index.md
git commit -m "docs: define ci performance gate policy and required checks"
```

### Task 6: Ensure Required Status Check Names Stay Stable

**Files:**
- Modify: `.github/workflows/verify.yml`
- Modify: `.github/workflows/security.yml`
- Modify: `.github/workflows/perf-lint.yml`
- Test: `rg` on workflow `name` fields

**Step 1: Write the failing test**

```bash
rg -n '^name:\s*(verify|Security|perf-lint)$' .github/workflows/verify.yml .github/workflows/security.yml .github/workflows/perf-lint.yml
```

**Step 2: Run test to verify it fails**

Run: same command as Step 1  
Expected: mismatch if names are not exactly the intended required check names.

**Step 3: Write minimal implementation**

Normalize workflow names to stable check labels used in branch protection:

```yaml
name: verify
name: security
name: perf-lint
```

**Step 4: Run test to verify it passes**

Run:

```bash
rg -n '^name:\s*verify$' .github/workflows/verify.yml
rg -n '^name:\s*security$' .github/workflows/security.yml
rg -n '^name:\s*perf-lint$' .github/workflows/perf-lint.yml
```

Expected: all checks pass with exactly one canonical name each.

**Step 5: Commit**

```bash
git add .github/workflows/verify.yml .github/workflows/security.yml .github/workflows/perf-lint.yml
git commit -m "ci: stabilize required status check names"
```

### Task 7: Final Verification and Branch Protection Hand-off

**Files:**
- Modify: `docs/references/ci-performance-gates.md`
- Test: local gate commands + GitHub branch protection query

**Step 1: Write the failing test**

```bash
gh api repos/chumyin/loongclaw/branches/main/protection --jq '{required_status_checks,required_pull_request_reviews}'
```

**Step 2: Run test to verify it fails or reveals missing checks**

Run: same command as Step 1  
Expected: either auth/setup failure (needs maintainer context) or missing required check names.

**Step 3: Write minimal implementation**

Document and apply required checks in GitHub repo settings:

```text
Required checks:
- verify
- security
- perf-lint

Plus release governance checks as already defined.
```

Record final required-check list in docs.

**Step 4: Run test to verify it passes**

Run:

```bash
gh api repos/chumyin/loongclaw/branches/main/protection --jq '.required_status_checks.contexts'
```

Expected: includes `verify`, `security`, `perf-lint`.

**Step 5: Commit**

```bash
git add docs/references/ci-performance-gates.md
git commit -m "governance: align branch protection with ci performance checks"
```

### Final Verification Gate

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
task perf:lint
bash scripts/check-docs.sh
```

Expected:
- all commands exit 0.
- PR-path checks remain fast while full benchmark is reserved for nightly/release preflight.
