# Sessions Send Implementation Plan

**Goal:** add a truthful outbound control primitive by introducing `sessions_send` for known
channel-backed root sessions, without pretending LoongClaw already supports live subagent steering
or target-session inbox execution.

**Architecture:** keep the channel-specific delivery logic in `crates/app/src/channel`, but add a
new async app tool path in the dispatcher. The tool validates the target through the session
registry, resolves the channel-backed session id, uses an injected sender for delivery, and records
an operational session event without modifying transcript history.

## Task 1: Extend config and tool catalog

Add:

- `tools.messages.enabled`
- `sessions_send` tool descriptor and provider schema
- runtime-tool-view gating so the tool is hidden unless explicitly enabled

Acceptance criteria:

- messaging is disabled by default
- enabling `tools.messages.enabled` exposes `sessions_send` in root tool views only

## Task 2: Add failing tests first

Add tests for:

- config default and TOML round-trip
- provider schema visibility for `sessions_send`
- root send success with fake delivery transport
- rejection of delegate-child callers or unsupported targets

Acceptance criteria:

- tests fail before implementation for the expected reasons:
  - config field missing
  - tool schema missing
  - runtime path not wired

## Task 3: Implement async dispatcher path

Add:

- constrained `sessions_send` request parsing and target validation
- dispatcher support for async outbound delivery
- injected sender abstraction for tests
- control-event persistence on successful send

Acceptance criteria:

- `sessions_send` never appends transcript rows
- `sessions_send` only targets known root sessions with supported channel prefixes
- delivery uses existing channel adapters rather than reimplementing per-channel HTTP logic

## Task 4: Document the surface

Update:

- `docs/product-specs/index.md`
- `docs/roadmap.md`
- this design and implementation-plan pair

Acceptance criteria:

- product spec explicitly captures the constrained outbound message scope
- roadmap notes that LoongClaw now has a real outbound operator primitive without live child steer

## Verification

Run:

```bash
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test -p loongclaw-app tools:: conversation:: config::runtime:: -- --nocapture --test-threads=1
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo test --workspace --all-features -- --test-threads=1
/Users/chum/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo fmt --all --check
git diff --check
```

