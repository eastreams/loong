# Quality Score

Per-crate quality grades. Updated after each major milestone. Grades identify where to focus improvement effort.

**Last updated:** 2026-03-08 (post-Phase 3)

## Grading Scale

- **A** — Solid. Well-tested, stable API, documented.
- **B** — Adequate. Functional, some gaps.
- **C** — Needs attention. Known issues or missing coverage.
- **D** — Known gap. Placeholder or incomplete.

## Crate Scores

| Crate | Test Coverage | API Stability | Doc Quality | Tech Debt | Overall |
|-------|:---:|:---:|:---:|:---:|:---:|
| contracts | A | A | B | A | A |
| kernel | A | A | B | B | A |
| protocol | B | B | C | B | B |
| app | B | B | B | C | B |
| spec | B | B | C | B | B |
| bench | C | B | D | B | C |
| daemon | A | A | B | A | A |

## Notable Gaps

- **protocol** doc quality (C): reference docs exist but no architectural overview of transport/routing model
- **app** tech debt (C): D3 (sync->async bridge) and D4 (build_messages bypass) from [tech-debt-tracker](exec-plans/tech-debt-tracker.md)
- **bench** doc quality (D): no user-facing documentation for running benchmarks outside of scripts
- **spec** doc quality (C): spec-runner reference exists but lacks examples for common workflows

## History

| Date | Change |
|------|--------|
| 2026-03-08 | Initial scores after Phase 0-3 restructure |
