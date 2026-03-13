# Design Documents Index

Catalog of design documents and architectural decisions.

## Active Design Documents

| Document | Scope | Status |
|----------|-------|--------|
| [Core Beliefs](core-beliefs.md) | Engineering principles and taste enforcement | Living |
| [Layered Kernel Design](layered-kernel-design.md) | L0-L9 kernel layer specification and boundary rules | Living |
| [Provider Runtime Roadmap](provider-runtime-roadmap.md) | Provider/runtime evolution strategy | Active |
| [ACP/ACPX Pre-Embed](acp-acpx-preembed.md) | Advanced cryptographic primitives | Active |
| [Harness Engineering](harness-engineering.md) | Environment design for agent-driven development | Active |

## Tracked Deviations

| ID | Description | Status | Tracking |
|----|-------------|--------|----------|
| D1 | `spec -> app` dependency (transitional runtime coupling) | Open | Must be retired by architecture refactor |

## Decision Log

Decisions from the research repository that are accepted into this codebase:

| ID | Decision | Status |
|----|----------|--------|
| D-001 | Zircon-style capability model (handle + rights, membrane revocation) | Accepted |
| D-002 | Hybrid agent lifecycle (Actix states + Erlang/OTP supervision) | Accepted |
| D-015 | OAuth 2.1 external + capability internal auth | Accepted |
| D-016 | MemoryStore trait (4 typed async methods) | Accepted |
| D-017 | MemoryScope enum (Task, Session, Agent, Global) | Accepted |
| D-018 | SQLite + FTS5 default backend (WAL, feature-gated sqlite-vec) | Accepted |
| D-019 | Mandatory provenance fields | Accepted |
| D-020 | Configurable trust scoring (Tier 0-3) | Accepted |
| D-021 | Blake3 content hashing | Accepted |
| D-022 | Capability-scoped deletion (revocation cascades) | Accepted |
