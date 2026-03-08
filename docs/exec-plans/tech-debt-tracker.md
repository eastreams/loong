# Tech Debt Tracker

Known technical debt items, tracked for continuous paydown. Items are promoted from roadmap discussion or discovered during development.

## Active Debt

| ID | Item | Severity | Crate(s) | Origin |
|----|------|----------|----------|--------|
| D1 | Phase 3 primitives not wired into production paths | Medium | kernel, app | Phase 3 review |
| D2 | InMemoryAuditSink loses events on restart | High | kernel, app | Copilot review |
| D3 | `persist_turn` uses `block_in_place` (sync->async bridge) | Medium | app | Copilot review |
| D4 | `build_messages` memory window bypasses kernel | Medium | app | Phase 2 TODO |
| D5 | InMemoryAuditSink grows unboundedly | Low | kernel | Code review |

## Resolved Debt

| ID | Item | Resolution | Date |
|----|------|------------|------|
| — | NoopAuditSink in bootstrap paths | Switched to InMemoryAuditSink | 2026-03-08 |
| — | SqliteMemoryAdapter naming inconsistency | Renamed to MvpMemoryAdapter | 2026-03-08 |
| — | KernelContext data redundancy (duplicate pack_id/agent_id) | Removed duplicates, added accessors | 2026-03-08 |

## Process

- New debt items are added here when discovered during reviews or development.
- Each item has a severity (High/Medium/Low) and affected crate(s).
- High-severity items should be resolved before new feature work.
- When resolved, move to the Resolved section with date and resolution summary.
- See also: [Roadmap Discussion Items](../roadmap.md#discussion-post-mvp-foundation-items)
