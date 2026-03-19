# GitHub Label Taxonomy Refresh Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace prefixed `area:*` / `domain:*` labels with clearer unprefixed labels, remove
automatic `rust` labeling, and make one taxonomy manifest drive GitHub label automation, issue
forms, and collaboration docs.

**Architecture:** Store the label taxonomy in one checked-in JSON manifest. Add a small Python
generator/checker that renders the GitHub artifacts from that manifest. Keep the GitHub workflow
simple by reading the generated workflow file rather than adding a separate custom action.

**Tech Stack:** JSON, Python 3 standard library, GitHub Actions YAML, issue-form YAML, repository
shell regression tests.

---

## Task 1: Add a failing taxonomy regression test

**Files:**
- Create: `scripts/test_sync_github_labels.sh`
- Create: `scripts/sync_github_labels.py`

**Step 1: Write the failing test**

Create a shell regression test that expects a generator/checker script to:

- reject prefixed `area:` and `domain:` managed labels
- reject automatic `rust` labeling
- keep generated files in sync with one taxonomy source

**Step 2: Run test to verify it fails**

Run: `bash scripts/test_sync_github_labels.sh`
Expected: fail because the generator/checker script and manifest do not exist yet.

## Task 2: Add the taxonomy manifest and generator

**Files:**
- Create: `.github/label_taxonomy.json`
- Create: `scripts/sync_github_labels.py`

**Step 1: Define the taxonomy**

Add the managed surface labels, managed domain labels, shared labels, surface dropdown options, and
path globs to one JSON manifest.

**Step 2: Implement generation**

Make the script render:

- `.github/labeler.yml`
- `.github/workflows/labeler.yml`
- `.github/ISSUE_TEMPLATE/bug_report.yml`
- `.github/ISSUE_TEMPLATE/feature_request.yml`
- `.github/ISSUE_TEMPLATE/docs_improvement.yml`
- `docs/references/github-collaboration.md`

**Step 3: Support verification mode**

Add a `--check` mode that exits non-zero if any generated file differs from the rendered output.

## Task 3: Regenerate GitHub artifacts with the new taxonomy

**Files:**
- Modify: `.github/labeler.yml`
- Modify: `.github/workflows/labeler.yml`
- Modify: `.github/ISSUE_TEMPLATE/bug_report.yml`
- Modify: `.github/ISSUE_TEMPLATE/feature_request.yml`
- Modify: `.github/ISSUE_TEMPLATE/docs_improvement.yml`
- Modify: `docs/references/github-collaboration.md`

**Step 1: Rename labels**

Move the managed subsystem labels from `area:*` to unprefixed surface names and move the roadmap
labels from `domain:*` to unprefixed domain names.

**Step 2: Remove `rust` automation**

Delete the path-based `rust` label and stop the workflow from managing that label.

**Step 3: Improve naming and docs**

Rename the issue-form dropdown from `Area` to `Surface` and update docs to explain surface labels
vs. domain labels.

## Task 4: Wire the new check into repo governance

**Files:**
- Modify: `Taskfile.yml`
- Modify: `.github/workflows/ci.yml`

**Step 1: Add a dedicated check**

Expose the label-taxonomy check through a repo task so local verification and CI use the same
command.

**Step 2: Add CI coverage**

Run the new regression test and the generator `--check` mode in the governance job.

## Task 5: Verify and publish

**Files:**
- Modify: GitHub issue / PR artifacts after code lands

**Step 1: Run focused governance checks**

Run:

- `python3 scripts/sync_github_labels.py --check`
- `bash scripts/test_sync_github_labels.sh`

**Step 2: Run broader repo verification for touched surfaces**

Run the relevant local governance checks, including the updated CI governance coverage.

**Step 3: Prepare GitHub delivery**

Open or reuse a tracking issue, push the branch to the operator fork, and open a PR with an
explicit closing clause and validation notes.
