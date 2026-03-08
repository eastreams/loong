# Docs Restructure Design — Harness Engineering Patterns

**Goal:** Restructure `docs/` to follow progressive disclosure patterns from OpenAI's harness engineering approach. Agents start with a small entry point (CLAUDE.md as map) and are taught where to look next, rather than being overwhelmed up front.

**Motivation:** The OpenAI article demonstrated that agent-first codebases need structured, indexed, mechanically-verified documentation as the system of record. Our current flat `docs/` layout mixes completed plans with active work and lacks quality tracking.

## Target Structure

```
CLAUDE.md              (table of contents / map — ~100 lines)
AGENTS.md              (mirror of CLAUDE.md)
ARCHITECTURE.md        (crate map, data flow, "where does X live")
CONTRIBUTING.md        (contribution tracks, recipes)
SECURITY.md            (security reporting)
docs/
├── design-docs/
│   ├── index.md
│   ├── core-beliefs.md
│   ├── layered-kernel-design.md    (moved from docs/)
│   └── versioning-policy.md        (moved from docs/)
├── exec-plans/
│   ├── active/                     (empty — no active plans)
│   ├── completed/
│   │   ├── 2026-03-08-mvp-foundation-restructure.md
│   │   └── 2026-03-08-kernel-primitives-phase3.md
│   └── tech-debt-tracker.md        (D1-D5 from roadmap)
├── product-specs/
│   └── index.md                    (placeholder)
├── references/
│   ├── mvp-foundation-architecture.md
│   ├── mvp-quickstart.md
│   ├── plugin-manifest-format.md
│   ├── plugin-runtime-governance.md
│   ├── programmatic-pressure-benchmark.md
│   ├── programmatic-tool-call.md
│   ├── spec-runner.md
│   └── status-roadmap-mvp-2026-03-08.md
├── index.md                        (updated as new TOC)
├── roadmap.md                      (stays — cross-cutting)
├── QUALITY_SCORE.md                (crate quality grades)
└── RELIABILITY.md                  (reliability expectations)
```

## Migration Plan

| Current | Target | Action |
|---------|--------|--------|
| `docs/plans/*.md` | `docs/exec-plans/completed/` | Move (both plans are completed) |
| `docs/layered-kernel-design.md` | `docs/design-docs/` | Move |
| `docs/versioning-policy.md` | `docs/design-docs/` | Move |
| `docs/reference/*.md` | `docs/references/` | Move (rename dir) |
| `docs/index.md` | `docs/index.md` | Rewrite as progressive disclosure TOC |
| `docs/roadmap.md` | `docs/roadmap.md` | Keep in place |

## New Files

| File | Purpose |
|------|---------|
| `docs/design-docs/index.md` | Index of design docs with verification status |
| `docs/design-docs/core-beliefs.md` | 10 golden principles for agent + human work |
| `docs/exec-plans/tech-debt-tracker.md` | D1-D5 items from roadmap discussion |
| `docs/product-specs/index.md` | Placeholder with structure for future use |
| `docs/QUALITY_SCORE.md` | Letter grades per crate (coverage, stability, docs, debt) |
| `docs/RELIABILITY.md` | Reliability expectations and invariants |

## Core Beliefs (for core-beliefs.md)

1. Kernel-first — all paths route through capability/policy/audit
2. No breaking changes — additive primitives only
3. Capability-gated by default — valid token required for all operations
4. Audit everything security-critical — silent drops are bugs
5. 7-crate DAG, no cycles — dependency direction is non-negotiable
6. Tests are the contract — untested behavior doesn't exist
7. Boring technology preferred — composable, agent-legible dependencies
8. Repository is the system of record — if it's not in repo, it doesn't exist
9. Enforce mechanically, not manually — encode taste into tooling
10. YAGNI ruthlessly — minimum complexity for current task

## Quality Score Format

Per-crate letter grades (A/B/C/D) across: Test Coverage, API Stability, Doc Quality, Tech Debt, Overall.

## CLAUDE.md Update

Rewrite as a map (~100 lines) pointing to deeper sources of truth. Progressive disclosure: agents read CLAUDE.md first, then follow pointers to design-docs, exec-plans, references as needed.

## Success Criteria

- All existing doc links in ARCHITECTURE.md and CONTRIBUTING.md still resolve
- `docs/index.md` serves as progressive disclosure entry point
- No orphaned files in old locations
- CLAUDE.md and AGENTS.md updated and mirrored
