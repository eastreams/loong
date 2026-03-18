# GitHub Collaboration Reference

This document defines the active GitHub collaboration baseline for the `dev` branch.

## Active Branch Model

- `dev` is the active integration branch for day-to-day OSS work.
- Contributors should branch from `dev` and target `dev` with normal pull requests.
- `main` is the stable promotion branch. Only reviewed `dev` changes should move into `main`.
- `release` and `release/*` branches are optional release-hardening lanes. When used, they should
  only receive reviewed `main` changes.
- Promotion pull requests into `main` should come from `dev` and stay focused on stabilised work,
  not mixed feature development.

## Promotion and Release Rhythm

- Maintainers aim to promote a stable slice from `dev` into `main` on a regular cadence.
- When a dedicated release-hardening lane is needed, maintainers may promote `main` into `release`
  or `release/*` before tagging.
- Exact timing depends on validation status, scope completion, and operational readiness.
- Releases are published from stable promotion points when the shipped slice is complete enough to
  support a tagged release. Not every `dev -> main` promotion must become a public release.

## CI and Promotion Gates

- `CI`, `CodeQL`, and `Security` validate pull requests and pushes for `dev`, `main`, `release`,
  and `release/*`.
- `perf-lint` uses the same branch set but stays path-scoped to workflow and benchmark-sensitive
  files.
- `enforce-dev-to-main` closes promotion PRs into `main` when the source branch is not `dev`.
- `enforce-main-to-release` closes promotion PRs into `release` lanes when the source branch is
  not `main`.
- The stable branch-protection check is `build`, the aggregate job in `.github/workflows/ci.yml`.

## Intake Routes

| Need | Route | Why |
| --- | --- | --- |
| Reproducible runtime defect | Bug report form | Captures severity, regression status, repro, runtime context, and evidence. |
| New capability or behavior change | Feature request form | Captures problem statement, acceptance criteria, rollout notes, and scope boundaries. |
| Missing, wrong, or confusing docs | Documentation improvement form | Captures branch-model drift, workflow gaps, and concrete doc references. |
| Setup question or general troubleshooting | GitHub Discussions, Discord, Telegram, or the community spaces you already use such as Feishu and WeChat | Keeps support traffic out of the issue queue and lets people ask where they already participate. |
| Direct contributor introduction or “where could I help?” conversation | [contact@loongclaw.ai](mailto:contact@loongclaw.ai) | Works well for async introductions, timezone context, and matching contributors to work that fits their strengths. |
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

- External contributors should normally target `dev`.
- Promotion pull requests into `main` should come from `dev` and stay focused on stabilised work,
  not mixed feature development.
- Link the tracking issue in the PR body and include an explicit closing clause when the PR is meant to resolve it.
- Keep the PR scoped to one logical change stream.
- Fill in the PR template with changed areas, risk track, validation commands, and reviewer focus.
- If the change is Track B, include rollout and rollback notes directly in the PR body.

## Default Branch Notes

GitHub serves public issue forms from the default branch. In this repository, the default branch is
`dev`, which is also the active collaboration baseline. Keep contributor-facing templates, contact
links, and references aligned on `dev` so public intake stays consistent with the actual review
flow.
