# GitHub Collaboration Reference

This document defines the active GitHub collaboration baseline for the `alpha-test` branch.

## Active Branch Model

- `alpha-test` is the active integration branch for day-to-day OSS work.
- Contributors should branch from `alpha-test` and target `alpha-test` with normal pull requests.
- `main` is the promotion branch. Only reviewed `alpha-test` changes should move into `main`.
- The current default branch setting still lags this collaboration model. Until the default branch changes, or the same `.github/ISSUE_TEMPLATE` content is mirrored into `dev`, treat the docs and templates on `alpha-test` as the review source of truth.

## Intake Routes

| Need | Route | Why |
| --- | --- | --- |
| Reproducible runtime defect | Bug report form | Captures severity, regression status, repro, runtime context, and evidence. |
| New capability or behavior change | Feature request form | Captures problem statement, acceptance criteria, rollout notes, and scope boundaries. |
| Missing, wrong, or confusing docs | Documentation improvement form | Captures branch-model drift, workflow gaps, and concrete doc references. |
| Setup question or general troubleshooting | GitHub Discussions / Discord | Keeps support traffic out of the issue queue. |
| Security vulnerability | Private security advisory | Avoids publishing sensitive details in public issues. |

## Managed Labels

### Area Labels

These labels intentionally stay lightweight. They provide routing value without copying the oversized taxonomies used by much larger projects.

| Label | Meaning |
| --- | --- |
| `area: kernel` | Kernel policy, approvals, and audit surfaces |
| `area: contracts` | Shared contract and type surfaces |
| `area: protocol` | Protocol crate and wire-level behavior |
| `area: spec` | Architecture boundaries, specs, and design docs |
| `area: daemon` | Daemon binary, CLI entrypoints, and install flow |
| `area: providers` | Provider routing, profile selection, and transport behavior |
| `area: tools` | Tool runtime, policy adapters, and tool catalog behavior |
| `area: browser` | Browser automation surfaces |
| `area: channels` | Channel adapters and integrations |
| `area: memory` | Memory system, context assembly, and persistence flow |
| `area: conversation` | Conversation runtime, session flow, and prompt assembly |
| `area: config` | Runtime config parsing, schema, and defaults |
| `area: acp` | ACP manager, binding, routing, and control plane surfaces |
| `area: migration` | Onboarding, legacy import, and migration flow |
| `area: docs` | Contributor docs, references, and collaboration guidance |
| `area: ci` | CI, workflows, release automation, and governance scripts |

### Size Labels

| Label | Threshold |
| --- | --- |
| `size: XS` | 0-50 changed lines |
| `size: S` | 51-200 changed lines |
| `size: M` | 201-500 changed lines |
| `size: L` | 501-1000 changed lines |
| `size: XL` | More than 1000 changed lines |

### Existing General Labels

The collaboration baseline continues to use the existing general labels for issue type and common routing:

- `bug`
- `enhancement`
- `documentation`
- `question`
- `help wanted`
- `good first issue`
- `duplicate`
- `invalid`
- `wontfix`
- `triage`
- `dependencies`
- `github_actions`
- `rust`

## Automation Rules

- The `labeler` workflow ensures the managed `area:*` and `size:*` labels exist.
- Pull requests receive path-based labels from `.github/labeler.yml`.
- Pull requests also receive a single `size:*` label based on total added plus removed lines.
- Issue forms with an `Area` dropdown sync to one managed `area:*` label.
- Choosing `Unknown / needs triage` keeps the issue in `triage` without forcing an `area:*` label.
- Maintainers can run `workflow_dispatch` on `labeler` to backfill labels after merging the workflow.

## Pull Request Expectations

- Link the tracking issue in the PR body and include an explicit closing clause when the PR is meant to resolve it.
- Keep the PR scoped to one logical change stream.
- Fill in the PR template with changed areas, risk track, validation commands, and reviewer focus.
- If the change is Track B, include rollout and rollback notes directly in the PR body.

## Current Limitation

GitHub serves public issue forms from the default branch. At the moment, the repository default branch is not aligned with the active `alpha-test` contribution flow. That means the improved forms in `alpha-test` will not become public until one of these happens:

1. The repository default branch is changed to `alpha-test`.
2. The `.github/ISSUE_TEMPLATE` changes are mirrored into `dev`.

Until then, this document and [CONTRIBUTING.md](../../CONTRIBUTING.md) are the authoritative guidance for contributors and maintainers reviewing `alpha-test`.
