# Audit Repair Command

**Goal:** Add `loongclaw audit repair` to rebuild integrity sidecars for legacy journals (#876).

**Architecture:** New `Repair` variant in `AuditCommands` enum (daemon), new `repair_jsonl_audit_journal()` function (kernel). Mirrors existing `verify` pattern.

## Repair Semantics

| Verify finding | Repair action |
|---|---|
| missing integrity envelope | **Repair**: compute and inject SHA256 hash chain |
| healthy (already has valid integrity) | **Skip**: report as already healthy |
| prev_hash mismatch (after legacy prefix repair) | **Re-seal**: rebuild chain with new prev_hash |
| entry_hash vs event data mismatch | **Refuse**: report as tampered, do not reseal |

Repair writes a new journal file (`.jsonl.repair-tmp`), then atomically renames. Original is never modified in-place. Temp file is cleaned up on rename failure. **Must be run while daemon is stopped** (running `JsonlAuditSink` holds stale file handle).

## Result Type

```rust
pub struct AuditRepairReport {
    pub total_events: usize,
    pub repaired_events: usize,    // events that got new integrity envelopes
    pub already_valid_events: usize, // events that already had valid integrity
    pub outcome: AuditRepairOutcome,
}

pub enum AuditRepairOutcome {
    Healthy,              // journal already fully valid, nothing to repair
    Repaired,             // legacy events got integrity envelopes
    Refused { line: usize, reason: String }, // tampered/mismatched, cannot repair
}
```

## Files

| File | Change |
|---|---|
| `crates/kernel/src/audit.rs` | Add `repair_jsonl_audit_journal()`, `AuditRepairReport`, `AuditRepairOutcome` |
| `crates/daemon/src/audit_cli.rs` | Add `Repair` variant to `AuditCommands`, handler, JSON/text rendering |

## Algorithm

1. Read journal line by line, rebuild hash chain:
   - Event missing integrity → compute entry_hash from event + prev_hash, inject envelope
   - Event has integrity, chain matches, no prior repairs → keep as-is
   - Event has integrity, prior entries were repaired → re-seal with rebuilt prev_hash
   - Event entry_hash doesn't match its own event data → `Refused` (tampering)
2. If no events were repaired → return `Healthy`
3. Write repaired lines to `.jsonl.repair-tmp` temp file
4. Atomic rename temp → original
5. If rename fails, remove temp file and return error

## Out of Scope

- Background auto-repair
- Journal rotation/pruning
- Resealing tampered journals
