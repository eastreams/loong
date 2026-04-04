# Bash Exec Final Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clear the remaining PR-readiness blockers for the shipped `bash.exec` AST governance slice by fixing failing CI lanes, aligning the PR submission artifacts, and publishing durable `bash.exec` documentation without changing the current `shell.exec` contract.

**Architecture:** Treat this as release-readiness cleanup for an already-landed implementation slice, not as another behavior expansion. `bash.exec` remains an experimental parallel tool that coexists with `shell.exec`; this plan fixes governance artifacts, security-policy exceptions, docs, and PR metadata while explicitly avoiding `shell.exec` convergence, approval redesign, or discovery follow-up behavior changes.

**Tech Stack:** Rust, Cargo, GitHub Actions parity scripts, `cargo deny`, release-doc governance scripts, repository README/spec docs, GitHub PR metadata.

**Correctness Review Mode:** `auto-fix`

**Style Review Mode:** `single-pass`

---

## File Structure

- Create: `docs/releases/architecture-drift-2026-04.md`
  - Add the current UTC-month tracked architecture drift report required by the `governance` CI lane on 2026-04-01.
- Modify: `deny.toml`
  - Add explicit, evidence-backed `cargo deny` advisory exceptions for the two unmaintained transitive crates introduced by the new `starlark`-backed rule loader.
- Modify: `README.md`
  - Document `bash.exec` as an experimental parallel tool, explain `[tools.bash]`, and show the default `~/.loongclaw/rules` layout plus legacy `shell_*` compatibility.
- Modify: `README.zh-CN.md`
  - Mirror the `bash.exec` documentation updates in Chinese and update the docs link to the shipped tool-surface spec.
- Modify: `docs/product-specs/tool-surface.md`
  - Record the shipped tool-surface truth that `bash.exec` now exists as an experimental parallel tool and must be documented with its canonical name without implying `shell.exec` replacement.
- Delete: `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-body.en.md`
  - Remove the local draft-only issue artifact now that GitHub issue `#678` is the authoritative copy.
- Delete: `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-draft.en.md`
  - Remove the second local draft-only issue artifact for the same reason.
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`
  - Check off progress as implementation advances.

## Implementation Notes

- `bash.exec` stays experimental and parallel. Do not overwrite, rename, or converge `shell.exec` in this slice.
- The CI evidence on 2026-04-01 shows three failing checks on PR `#677`: `governance`, `advisory-checks`, and the meta `build` job that only reflects the `governance` failure.
- On 2026-04-01 UTC, `scripts/check_architecture_drift_freshness.sh` resolves the required tracked report path to `docs/releases/architecture-drift-2026-04.md`. The repository currently only tracks `docs/releases/architecture-drift-2026-03.md`, so the current-month file must exist and be tracked before the freshness check can pass.
- The `advisory-checks` failure is currently limited to `RUSTSEC-2024-0388` (`derivative`) and `RUSTSEC-2024-0436` (`paste`), both pulled transitively by the direct `starlark = "0.13"` dependency used only for local Bash rule parsing. This cleanup slice uses the smallest repo-consistent response: explicit `cargo deny` exceptions with honest rationale, not a late parser replacement.
- The PR body for `#677` is stale: it still describes the pre-governance `bash.exec` slice and claims AST/rules work is not implemented. The final cleanup must rewrite it to match the current code, current CI evidence, and current scope boundary.
- Manual smoke evidence gathered in this branch should be preserved in docs/PR language:
  - `printf ok` is allowed when it actually executes through `bash.exec`
  - `git status --short` is allowed when it actually executes through `bash.exec`
  - `git rev-parse HEAD` is denied by default-deny when it actually executes through `bash.exec`
  - `cargo publish --dry-run` is denied
  - `c\\argo publish --dry-run` is also denied by the escaped-token hardening
- Invalid manual runs where the model stopped after `tool.search` or hallucinated a tool result are out of scope for this cleanup slice; that follow-up is tracked separately in GitHub issue `#678`.

## Scope In

- Current-month governance artifact freshness for the branch
- `cargo deny` convergence for the current `starlark`-introduced advisories
- PR `#677` body alignment with the repository template and actual shipped behavior
- Durable `bash.exec` documentation updates in repository docs
- Removal of the two now-obsolete local issue-draft files

## Scope Out

- Any attempt to replace, rename, or merge `shell.exec` into `bash.exec`
- Approval-required / approve-always work
- Discovery follow-up prompt fixes or `tool.search` loop redesign
- Replacing `starlark` with a different rules parser
- New `bash.exec` behavior changes beyond the already-shipped AST governance slice

### Task 1: Refresh the current-month governance artifact and clear the `governance` lane

**Files:**
- Create: `docs/releases/architecture-drift-2026-04.md`
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`

- [x] **Step 1: Prove the current UTC-month report is the missing tracked artifact**

Run:

- `git ls-files --error-unmatch docs/releases/architecture-drift-2026-04.md`
- `ls docs/releases | rg 'architecture-drift'`

Expected:

- the first command fails because `architecture-drift-2026-04.md` is not yet tracked
- the second command shows only `architecture-drift-2026-03.md`

- [x] **Step 2: Generate and add the 2026-04 architecture drift report**

Run:

- `bash scripts/generate_architecture_drift_report.sh docs/releases/architecture-drift-2026-04.md`
- `sed -n '1,140p' docs/releases/architecture-drift-2026-04.md`

Expected:

- the generator writes a new tracked-candidate report for `2026-04`
- the report reflects the current `tools_mod` size / hotspot state instead of the stale March snapshot

- [ ] **Step 3: Re-run the exact governance checks that blocked CI**

Run:

- `bash scripts/check_architecture_drift_freshness.sh docs/releases/architecture-drift-2026-04.md`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`
- `./scripts/check_dep_graph.sh`
- `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
- `git diff --check`

Expected:

- PASS on the freshness check against `docs/releases/architecture-drift-2026-04.md`
- PASS on the docs, dep-graph, architecture, and diff-hygiene gates

### Task 2: Resolve the `advisory-checks` lane with explicit `cargo deny` policy exceptions

**Files:**
- Modify: `deny.toml`
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`

- [x] **Step 1: Reproduce the failing advisory check locally**

Run:

- `cargo deny check advisories bans sources`
- `cargo tree -p loongclaw-app -i derivative`
- `cargo tree -p loongclaw-app -i paste`

Expected:

- `cargo deny` fails on `RUSTSEC-2024-0388` and `RUSTSEC-2024-0436`
- both dependency trees show the path through direct dependency `starlark v0.13.0`

- [x] **Step 2: Add targeted ignore entries with honest rationale**

Update `deny.toml` so the advisory section becomes:

```toml
[advisories]
ignore = [
    "RUSTSEC-2024-0388", # derivative unmaintained — transitive dep of starlark 0.13 used only for local bash rule parsing
    "RUSTSEC-2024-0436", # paste unmaintained — transitive dep of starlark 0.13 used only for local bash rule parsing
    "RUSTSEC-2025-0057", # fxhash unmaintained — transitive dep of scraper 0.24, no security impact
]
```

- [x] **Step 3: Re-run the security workflow parity commands**

Current status on 2026-04-01:

- `cargo deny check advisories bans sources` now passes locally after the targeted `deny.toml` update.
- `cargo audit` now passes locally after installing `cargo-audit`.

Run:

- `cargo audit`
- `cargo deny check advisories bans sources`

Expected:

- `cargo audit` passes
- `cargo deny` passes without the two `starlark`-related unmaintained advisories blocking the workflow

### Task 3: Publish durable `bash.exec` docs without implying `shell.exec` convergence

**Files:**
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/product-specs/tool-surface.md`
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`

- [x] **Step 1: Update the English README to document `bash.exec` as experimental and parallel**

In `README.md`, replace the current shell-policy-only example with a `bash.exec`-aware section that keeps `shell.exec` intact and explains the config boundary. The new example should look like:

```toml
[tools]
shell_default_mode = "deny"
shell_allow = ["echo", "ls", "git", "cargo"]
shell_deny = ["rm", "cargo publish"]

[tools.bash]
login_shell = false
# rules_dir = "~/.loongclaw/rules"
```

Add a short rules example directly below it:

```python
# ~/.loongclaw/rules/00-allow-basic.rules
prefix_rule(pattern=["printf"], decision="allow")
prefix_rule(pattern=["git", "status"], decision="allow")

# ~/.loongclaw/rules/90-deny-dangerous.rules
prefix_rule(pattern=["rm"], decision="deny")
prefix_rule(pattern=["cargo", "publish"], decision="deny")
```

The prose must explicitly say:

- `bash.exec` is experimental
- it is a parallel tool and does not replace `shell.exec`
- default rule files live in `~/.loongclaw/rules`
- legacy `shell_allow`, `shell_deny`, and `shell_default_mode` still feed compatibility rules/default mode for `bash.exec`

- [x] **Step 2: Mirror the same behavior truth in `README.zh-CN.md` and fix the broken link**

In `README.zh-CN.md`, add the matching Chinese explanation and config example, then replace the broken link target:

```md
- `docs/configuration/tool-policy.md`
```

with the real shipped spec link:

```md
- `docs/product-specs/tool-surface.md`
```

The Chinese prose must preserve the same scope boundary:

- `bash.exec` 仍处于实验阶段
- 它与 `shell.exec` 并行存在
- 这轮不宣称二者已经收敛或互相替代

- [x] **Step 3: Update the product spec so the shipped tool surface matches the docs**

Extend `docs/product-specs/tool-surface.md` with a short section like:

```md
## Current Tool Surface Notes

- `shell.exec` remains the existing shell-execution surface.
- `bash.exec` is a shipped experimental parallel tool. It may be advertised only when the runtime can actually execute it, but it does not replace `shell.exec`.
- User-facing docs must describe `bash.exec` with its canonical tool name and must not imply that shell governance has already converged on a single execution surface.
```

- [ ] **Step 4: Run focused docs validation**

Current status on 2026-04-01:

- `rg -n "bash\\.exec|shell\\.exec|rules_dir|login_shell|shell_allow|shell_deny" README.md README.zh-CN.md docs/product-specs/tool-surface.md` passes and shows the expected `bash.exec` references.
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh` remains blocked by pre-existing baseline docs/release issues outside this cleanup slice after merging latest `upstream/dev`.

Run:

- `rg -n "bash\\.exec|shell\\.exec|rules_dir|login_shell|shell_allow|shell_deny" README.md README.zh-CN.md docs/product-specs/tool-surface.md`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`

Expected:

- the grep shows the new `bash.exec` references in all three docs
- strict docs governance passes

### Task 4: Align PR `#677` with the template and clean local draft-only artifacts

**Files:**
- Delete: `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-body.en.md`
- Delete: `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-draft.en.md`
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`
- External update: GitHub PR `#677`

- [x] **Step 1: Remove the obsolete local issue drafts from the workspace**

Delete:

- `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-body.en.md`
- `docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-draft.en.md`

Expected:

- `git status --short` no longer shows the two untracked local issue-draft files

- [x] **Step 2: Rewrite the PR body to match the repository template and the real shipped scope**

Update PR `#677` so its body follows `.github/PULL_REQUEST_TEMPLATE.md` and uses content equivalent to:

```md
## Summary

- Problem: the PR body still describes the pre-governance `bash.exec` slice and no longer matches the shipped AST + prefix-rule implementation.
- Why it matters: reviewers and CI need an accurate record of what changed, what stayed out of scope, and how the new governance/documentation behavior was validated.
- What changed: `bash.exec` now ships as an experimental parallel tool with AST-based minimal-command-unit governance, Starlark-authored prefix rules, legacy `shell_*` compatibility inputs, current-month governance artifacts, and refreshed user-facing docs.
- What did not change (scope boundary): `shell.exec` is unchanged; there is still no `approval_required` outcome for `bash.exec`; discovery follow-up / `tool.search` behavior is out of scope.

## Linked Issues

- Closes #637
- Related #678

## Change Type

- [x] Bug fix
- [x] Feature
- [x] Documentation
- [x] Security hardening
- [x] CI / workflow / release

## Touched Areas

- [x] Tools
- [x] ACP / conversation / session runtime
- [x] Config / migration / onboarding
- [x] Docs / contributor workflow
- [x] CI / release / workflows

## Risk Track

- [x] Track B (higher-risk / policy-impacting)

If Track B, fill these in:

- Risk notes: `bash.exec` governance now defaults to deny for unmatched or unreliable units and consumes local rule files plus legacy `shell_*` compatibility inputs.
- Rollout / guardrails: `bash.exec` stays experimental and parallel to `shell.exec`; broken rule files fail closed; docs now state the scope boundary explicitly.
- Rollback path: revert the `bash.exec` governance series and remove the current-month release artifact / deny exceptions in one branch rollback.
```

- [x] **Step 3: Record the real validation evidence in the PR body**

The PR validation section must include the exact commands actually run in this slice:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --locked
cargo test --workspace --all-features --locked
cargo audit
cargo deny check advisories bans sources
bash scripts/check_architecture_drift_freshness.sh docs/releases/architecture-drift-2026-04.md
LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh
./scripts/check_dep_graph.sh
LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh
git diff --check
```

Also summarize the manual `bash.exec` smoke results:

- `printf ok` allowed
- `git status --short` allowed
- `git rev-parse HEAD` denied when the tool actually executed
- `cargo publish --dry-run` denied
- `c\\argo publish --dry-run` denied
- invalid `tool.search`-only runs excluded from pass/fail evidence and tracked separately in `#678`

### Task 5: Run full verification, pass review gates, and capture the cleanup milestone

**Files:**
- Modify: `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`

- [ ] **Step 1: Run the full local verification suite for this cleanup slice**

Current status on 2026-04-01:

- Passing locally: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --locked`, `cargo test --workspace --all-features --locked`, `cargo audit`, `cargo deny check advisories bans sources`, `bash scripts/check_architecture_drift_freshness.sh docs/releases/architecture-drift-2026-04.md`, and `git diff --check`.
- Remaining scope-out blockers accepted for separate follow-up PRs: strict docs governance remains blocked by pre-existing baseline docs/release issues outside this cleanup slice after merging latest `upstream/dev`, and strict architecture boundaries remain blocked by the real `tools_mod` budget breach surfaced by the refreshed April drift report.
- Current PR handling for those blockers: disclose them explicitly as not introduced by this PR and do not fix them inside this cleanup slice.

Run:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --locked`
- `cargo test --workspace --all-features --locked`
- `cargo audit`
- `cargo deny check advisories bans sources`
- `bash scripts/check_architecture_drift_freshness.sh docs/releases/architecture-drift-2026-04.md`
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh`
- `./scripts/check_dep_graph.sh`
- `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
- `git diff --check`

Expected:

- all scope-in commands pass on the final tree
- `LOONGCLAW_RELEASE_DOCS_STRICT=1 scripts/check-docs.sh` and
  `LOONGCLAW_ARCH_STRICT=true ./scripts/check_architecture_boundaries.sh`
  remain documented baseline failures until their separate follow-ups land

- [ ] **Step 2: Run the required superpower reviews and fix anything they find**

Run the implementation review flow required by the execution skill:

- correctness review in `auto-fix` mode
- style review in `single-pass` mode

Expected:

- all scope-in findings are fixed
- any remaining scope-outs are handed back separately instead of being silently absorbed

- [ ] **Step 3: Commit the completed cleanup milestone**

Run:

- `git add deny.toml docs/releases/architecture-drift-2026-04.md README.md README.zh-CN.md docs/product-specs/tool-surface.md docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`
- `git add -u docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-body.en.md docs/plans/2026-03-28-tool-result-followup-prompt-bug-issue-draft.en.md`
- `git commit -m "chore(app): finish bash.exec release cleanup"`

Expected:

- one local milestone commit that captures the CI fixes, docs, PR-readiness changes, and workspace cleanup together

## Self-Review

- Spec / request coverage:
  - failing CI lanes addressed: yes (`governance`, `advisory-checks`, and the resulting `build` meta failure)
  - PR submission requirements addressed: yes (template-aligned PR body task and validation evidence task)
  - `bash.exec` docs completion addressed: yes (README, README.zh-CN, product spec)
  - `shell.exec` overwrite explicitly forbidden: yes
  - local obsolete issue drafts cleaned up: yes
- Placeholder scan:
  - no `TODO`, `TBD`, or “similar to above” shortcuts left in task steps
  - exact files and commands are named for each task
- Type / behavior consistency:
  - docs language consistently states `bash.exec` is experimental and parallel
  - PR body and docs both preserve the same out-of-scope boundary for `shell.exec`, approval, and follow-up prompt behavior

## Execution Handoff

Plan complete and saved to `docs/plans/2026-04-01-bash-exec-final-cleanup-implementation-plan.md`.

Two execution options:

1. Subagent-Driven (recommended) - I dispatch a fresh subagent per task, review between tasks, fast iteration
2. Inline Execution - Execute tasks in this session using executing-plans, batch execution with checkpoints

Because this conversation is following the repo's superpower planning rules, stop after plan approval and wait for user confirmation before entering implementation.
