# Tech Debt Tracker

Living record of known architectural drift and technical debt. Updated as items are discovered or resolved.

## Active Debt

| ID | Description | Severity | Domain | Discovered |
|----|-------------|----------|--------|------------|
| TD-001 | D1: `spec -> app` dependency violates strict DAG | High | Architecture | 2026-03-11 |
| TD-002 | Policy engine only gates `shell.exec`; `file.read`/`file.write` bypass Rule of Two | High | Security (L1) | 2026-03-10 |
| TD-003 | `membrane` field on CapabilityToken exists but never checked in authorization | Medium | Security (L1) | 2026-03-10 |
| TD-004 | `call_depth` in PolicyContext never incremented, always 0 | Medium | Security (L1) | 2026-03-10 |
| TD-005 | Namespace struct is field-only, no scoped enforcement | Medium | Packs (L5) | 2026-03-10 |
| TD-006 | Audit events in-memory only, lost on restart | High | Observability (L4) | 2026-03-10 |
| TD-007 | No HMAC chain on audit events (tamper evidence absent) | High | Observability (L4) | 2026-03-10 |
| TD-008 | Context assembly hardcodes sliding window, no pluggable ContextEngine | Medium | Context/Memory | 2026-03-10 |
| TD-009 | Payload budget tracks characters, not tokens | Low | Context/Memory | 2026-03-10 |
| TD-010 | Memory has no scopes (Task/Session/Agent/Global), flat session_id only | Medium | Context/Memory | 2026-03-10 |
| TD-011 | No FTS5 index for full-text search on memory | Low | Context/Memory | 2026-03-10 |
| TD-012 | Plugin security scanner type exists but scanner logic absent | Medium | Integration (L6) | 2026-03-10 |
| TD-013 | WASM fuel metering / epoch interruption not implemented | Medium | Execution (L2) | 2026-03-10 |
| TD-014 | No mandatory provenance fields on memory entries (D-019 requires 10 fields) | Medium | Context/Memory | 2026-03-13 |
| TD-015 | No trust scoring on memory entries (D-020: Tier 0-3 calibration) | Low | Context/Memory | 2026-03-13 |
| TD-016 | No Blake3 content hashing for memory integrity (D-021) | Low | Context/Memory | 2026-03-13 |
| TD-017 | No tombstone audit trail for deletions (D-022: capability-scoped cascades) | Medium | Context/Memory | 2026-03-13 |
| TD-018 | No materialized views from event log (D-004) | Low | Runtime Core | 2026-03-13 |
| TD-019 | No process isolation for shell.exec (seccomp/Landlock/sandbox_init) | High | Security (L1) | 2026-03-13 |
| TD-020 | Audit signing key shares process with plugins — forgeable audit log | High | Security (L1) | 2026-03-13 |
| TD-021 | No observation masking when context nears limits | Medium | Context/Memory | 2026-03-13 |
| TD-022 | MemoryStore trait not implemented — using string dispatch instead of typed methods (D-016) | Medium | Context/Memory | 2026-03-13 |

## Resolved

| ID | Description | Resolution | Date |
|----|-------------|------------|------|
