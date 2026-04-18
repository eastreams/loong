# Quality Score

Domain grades for Loong. Updated periodically to track gaps, prioritize cleanup, and measure harness maturity.

## Domain Grades

| Domain | Grade | Last Reviewed | Gaps |
|--------|-------|---------------|------|
| Contracts (L0) | A | 2026-03-13 | `#[non_exhaustive]` applied; membrane field not yet enforced at runtime |
| Kernel Security (L1) | B+ | 2026-04-03 | Core tool paths now route through policy extensions; remaining gaps are non-uniform L1 coverage for connector/ACP/runtime-only analytics and explicit `Direct` compatibility lanes |
| Execution Planes (L2) | B | 2026-03-13 | Core/extension pattern solid; no WASM fuel metering yet |
| Orchestration (L3) | B | 2026-03-13 | HarnessBroker routes correctly; context-engine selection is pluggable, but richer engine implementations and broader runtime coverage are still limited |
| Observability (L4) | C+ | 2026-04-03 | Durable JSONL/fanout audit exists; remaining gaps are tamper-evident verification, query baseline, and richer export lanes |
| Vertical Packs (L5) | B | 2026-03-13 | Pack validation works; namespace struct exists but not enforced |
| Protocol (L5.5) | B+ | 2026-03-13 | Transport contracts and typed routing operational |
| Integration (L6) | B | 2026-03-13 | Plugin scanning works; hotplug lifecycle incomplete |
| Plugin IR (L7) | B- | 2026-03-13 | Bridge inference works; multi-language support limited |
| Self-Awareness (L8) | B- | 2026-03-13 | Snapshots generated but not continuous; no drift detection agent |
| Bootstrap (L9) | B | 2026-03-13 | Activation plans work; no policy-bounded bootstrap validation |
| Context/Memory | B | 2026-04-18 | Canonical, session-event, and workspace durable memory recall now share normalized FTS-backed retrieval with indexed `search_text` storage and Chinese tokenization support, but trust scoring, TTL policy, and richer derived-memory ranking are still limited |
| Documentation | A- | 2026-03-13 | Strong coverage across design docs, security, product sense, and quality tracking |
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
