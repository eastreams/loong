# User-Facing Setup Removal Polish Design

## Goal

Finish the `setup` removal from the current operator experience without rewriting historical
design records. After this change, current CLI help, onboarding feedback, install helpers, and
active README guidance should consistently describe `onboard` as the first-run path.

## Scope

In scope:

- current CLI help text emitted by `loongclaw --help`
- onboarding/preflight error strings shown to operators
- active README content and install helper wording
- regression tests that lock the user-facing wording boundary

Out of scope:

- historical records under `docs/plans/`
- internal code comments that are not emitted to operators
- broad terminology rewrites unrelated to the removed subcommand

## Approach

1. Add a regression test around CLI help output so `setup` does not reappear in current help text.
2. Update user-facing strings that still use `setup` as a first-run noun where it can imply the
   removed command still exists.
3. Keep changes narrowly scoped to active user surfaces so the branch remains easy to review and
   safe to land on `alpha-test`.

## User-Facing Decisions

- `onboard` remains the only first-run entrypoint.
- Generic wording like "setup" is replaced where it can create command confusion.
- `doctor` keeps its purpose but is described as diagnostics/config repair, not "setup diagnostics".
- Historical planning docs are intentionally left untouched to preserve design history.

## Validation

- unit/regression tests for CLI help text
- targeted daemon onboarding tests
- format, clippy, and the relevant package tests before completion
