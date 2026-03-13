# Execution Plans

This directory hosts agent-scoped execution plans that persist context across sessions.

## Structure

- `active/` — Plans currently in progress. Agents read and update these.
- `completed/` — Archived plans. Read-only reference.

## Relationship to `docs/plans/`

The `docs/plans/` directory contains **phase-based design and implementation plans** (the historical record of all planned work). This `exec-plans/` directory is for **agent working memory** — shorter-lived, task-scoped plans that agents create and consume during multi-step work.

Use `docs/plans/` for durable architectural plans.
Use `docs/exec-plans/active/` for in-flight agent tasks.

## Conventions

- Name format: `YYYY-MM-DD-<brief-topic>.md`
- Move to `completed/` when done (don't delete — agents may reference completed plans)
- Include progress checkboxes and decision logs within each plan
