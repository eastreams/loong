# Quality Score

Domain grades for LoongClaw. Updated periodically to track gaps, prioritize cleanup, and measure harness maturity.

## Domain Grades

| Domain | Grade | Last Reviewed | Gaps |
|--------|-------|---------------|------|
| Contracts (L0) | A | 2026-03-13 | `#[non_exhaustive]` applied; membrane field not yet enforced at runtime |
| Kernel Security (L1) | B+ | 2026-04-03 | Core tool execution now flows through capability checks and `PolicyExtensionChain`; remaining gaps are connector/ACP/runtime-only analytics and compatibility-seam closure |
| Execution Planes (L2) | B | 2026-03-13 | Core/extension pattern solid; no WASM fuel metering yet |
| Orchestration (L3) | B | 2026-03-13 | HarnessBroker routes correctly; context-engine selection is pluggable, but richer engine implementations and broader runtime coverage are still limited |
| Observability (L4) | B- | 2026-04-03 | Durable JSONL and fanout audit bootstraps exist; no tamper-evident chain, richer external sink adapters, or broader operator-facing audit evidence products yet |
| Vertical Packs (L5) | B | 2026-03-13 | Pack validation works; namespace struct exists but not enforced |
| Protocol (L5.5) | B+ | 2026-03-13 | Transport contracts and typed routing operational |
| Integration (L6) | B | 2026-03-13 | Plugin scanning works; hotplug lifecycle incomplete |
| Plugin IR (L7) | B- | 2026-03-13 | Bridge inference works; multi-language support limited |
| Self-Awareness (L8) | B- | 2026-04-03 | Monthly drift review artifacts exist, but continuous drift detection and automated reviewer guidance are still limited |
| Bootstrap (L9) | B | 2026-03-13 | Activation plans work; no policy-bounded bootstrap validation |
| Context/Memory | C | 2026-03-13 | SQLite turns only; no scopes, no provenance, no FTS5 |
| Documentation | A- | 2026-04-03 | Strong coverage across design docs, security, product sense, quality tracking, and AGT comparison context; still needs more release-grade benchmark and governance evidence summaries |
| CI/Enforcement | A | 2026-03-13 | 8 CI workflows, convention-engineering (14 files, 11 checks), check:harness mirror gate |
| Contributor Experience | A- | 2026-03-13 | Clear tracks and recipes; could add more examples |

## Grading Criteria

- **A**: Full test coverage, no known debt, documentation current, mechanical enforcement
- **B**: Adequate coverage, minor debt tracked, docs mostly current
- **C**: Coverage gaps, significant debt, stale or missing docs
- **D**: Minimal coverage, blocking debt, docs unreliable
- **F**: Untested, untracked, undocumented

## Harness Maturity Assessment

| Criterion | Status |
|-----------|--------|
| Agent entry point (AGENTS.md) | Present, 102 lines, mirrored with CLAUDE.md |
| Architecture defined with enforcement | Present, DAG + boundary checks + CI |
| Progressive disclosure hierarchy | Present, 3-tier structure |
| Mechanical enforcement | 8 CI workflows, convention-engineering (14 files, 11 content checks), check:harness, pre-commit |
| Quality tracked | This file |
| External context captured | Core beliefs principle #8 requires it |
