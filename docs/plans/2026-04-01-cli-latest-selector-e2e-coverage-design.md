# CLI Latest Selector End-to-End Coverage Design

Date: 2026-04-01
Issue: `#759`
PR: `#765`
Status: Proposed for the current task branch

## Problem

The current `latest` selector coverage proves three narrow layers:

1. repository selection finds the expected resumable root session
2. app-layer runtime bootstrap resolves `latest` into a concrete session id
3. daemon CLI parsing accepts the literal `latest` flag value

What is still missing is one higher-level proof that the resolved session id is actually consumed by
the downstream CLI surfaces that matter once bootstrap completes.

## Goal

Add the smallest end-to-end coverage slice that proves:

1. `latest` resolves through the real sqlite-backed runtime bootstrap
2. the resolved session id flows into the chat startup summary
3. the resolved session id is then used for real history reads

## Non-Goals

1. no new selector behavior
2. no process-level binary harness for `ask` or `chat`
3. no provider execution coverage
4. no new generic CLI session-selector abstraction

## Approaches Considered

### A. Add process-level `loongclaw chat/ask` integration tests

Pros:

- closest to a real operator invocation

Cons:

- requires substantially more harness setup for config, provider behavior, and interactive I/O
- increases flake risk without improving confidence on the specific selector handoff bug surface

### B. Extend app-layer runtime tests with real sqlite memory and downstream consumers

Pros:

- directly exercises the real selector bootstrap path already shared by `ask` and `chat`
- keeps changes local to the module that owns the behavior
- can prove both startup summary and history loading use the resolved session id

Cons:

- not a full OS-process execution path

### C. Add more repository or clap-only tests

Pros:

- cheapest change

Cons:

- does not cover the missing handoff from selector resolution into downstream CLI behavior

## Decision

Choose approach B.

The smallest correct move is to extend `crates/app/src/chat.rs` tests with real sqlite-backed
runtime initialization and then assert behavior at the first downstream consumers:

1. `build_cli_chat_startup_summary`
2. `load_history_lines`

This keeps the test on the exact ownership boundary where the selector is implemented, avoids a
large new harness, and meaningfully increases confidence that `latest` is not only parsed and
resolved, but actually used by CLI behavior after bootstrap.

## Validation Strategy

Minimum required validation for this slice:

1. add a failing async test that seeds multiple sessions and proves history comes from the resolved
   latest root session
2. add a failing summary-level test that proves the startup summary exposes the resolved session id
3. implement only the minimal code needed if the tests reveal an actual gap
4. rerun focused tests plus the full project verification already expected for this PR
