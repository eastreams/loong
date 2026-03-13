# Plans

High-level index for execution plans, tech debt, and planning conventions in LoongClaw.

## Active Execution Plans

Execution plans live in [`plans/`](plans/) with paired design/implementation splits per phase.

### Channel Architecture (Phases 1-13)

| Phase | Topic | Design | Implementation |
|-------|-------|--------|----------------|
| 1 | Reliability | [design](plans/2026-03-11-channel-reliability-design.md) | [impl](plans/2026-03-11-channel-reliability-phase-1.md) |
| 2 | Runtime | [design](plans/2026-03-11-channel-runtime-phase-2-design.md) | [impl](plans/2026-03-11-channel-runtime-phase-2.md) |
| 3 | Registry | [design](plans/2026-03-11-channel-registry-phase-3-design.md) | [impl](plans/2026-03-11-channel-registry-phase-3.md) |
| 4 | Runtime State | [design](plans/2026-03-11-channel-runtime-state-phase-4-design.md) | [impl](plans/2026-03-11-channel-runtime-state-phase-4.md) |
| 5 | Doctor Runtime | [design](plans/2026-03-11-channel-doctor-runtime-phase-5-design.md) | [impl](plans/2026-03-11-channel-doctor-runtime-phase-5.md) |
| 6 | Multiprocess | [design](plans/2026-03-11-channel-runtime-multiprocess-phase-6-design.md) | [impl](plans/2026-03-11-channel-runtime-multiprocess-phase-6.md) |
| 7 | Account Identity | [design](plans/2026-03-11-channel-account-identity-phase-7-design.md) | [impl](plans/2026-03-11-channel-account-identity-phase-7.md) |
| 8 | Multi-Account | [design](plans/2026-03-11-channel-multi-account-phase-8-design.md) | [impl](plans/2026-03-11-channel-multi-account-phase-8.md) |
| 9 | Account Hardening | [design](plans/2026-03-11-channel-account-hardening-phase-9-design.md) | [impl](plans/2026-03-11-channel-account-hardening-phase-9.md) |
| 10 | Default Routing | [design](plans/2026-03-11-channel-default-routing-phase-10-design.md) | [impl](plans/2026-03-11-channel-default-routing-phase-10.md) |
| 11 | Runtime Routing | [design](plans/2026-03-11-channel-runtime-routing-phase-11-design.md) | [impl](plans/2026-03-11-channel-runtime-routing-phase-11.md) |
| 12 | Context Runtime | [design](plans/2026-03-11-channel-runtime-context-phase-12-design.md) | [impl](plans/2026-03-11-channel-runtime-context-phase-12.md) |
| 13 | Serve Ownership | [design](plans/2026-03-11-channel-serve-ownership-phase-13-design.md) | [impl](plans/2026-03-11-channel-serve-ownership-phase-13.md) |

### Infrastructure Plans

| Plan | Design | Implementation |
|------|--------|----------------|
| CLI Name Unification | [design](plans/2026-03-11-cli-name-unification-design.md) | [impl](plans/2026-03-11-cli-name-unification.md) |
| Hybrid Runtime | [design](plans/2026-03-11-hybrid-runtime-design.md) | — |
| Memory Architecture | [design](plans/2026-03-11-loongclaw-memory-architecture-design.md) | [impl](plans/2026-03-11-loongclaw-memory-architecture-implementation.md) |
| Migration Nativeization | [design](plans/2026-03-11-loongclaw-migration-nativeization-design.md) | [impl](plans/2026-03-11-loongclaw-migration-nativeization-implementation.md) |
| Onboard Orchestrated Migration | [design](plans/2026-03-11-loongclaw-onboard-orchestrated-migration-design.md) | [impl](plans/2026-03-11-loongclaw-onboard-orchestrated-migration-implementation-plan.md) |
| Prompt Pack | [design](plans/2026-03-11-loongclaw-prompt-pack-design.md) | [impl](plans/2026-03-11-loongclaw-prompt-pack-implementation.md) |
| External Skills Runtime Closure | [design](plans/2026-03-12-external-skills-runtime-closure-design.md) | [impl](plans/2026-03-12-external-skills-runtime-closure.md) |
| Setup Removal Polish | [design](plans/2026-03-12-setup-removal-user-facing-polish-design.md) | [impl](plans/2026-03-12-setup-removal-user-facing-polish.md) |

## Tech Debt

See [Tech Debt Tracker](plans/tech-debt-tracker.md) for the living record of known architectural drift.

## Planning Conventions

- Every plan has a **design doc** (rationale, constraints, alternatives) and an **implementation doc** (tasks, progress, decisions)
- Plans are prefixed with date: `YYYY-MM-DD-<topic>-design.md` / `YYYY-MM-DD-<topic>.md`
- Completed plans remain in `plans/` as historical record
- External decisions must be transcribed into design docs or plans — if it's not in the repo, it doesn't exist for agents

## See Also

- [Roadmap](ROADMAP.md) — stage-based milestones
- [Design Docs Index](design-docs/index.md) — architectural decisions
