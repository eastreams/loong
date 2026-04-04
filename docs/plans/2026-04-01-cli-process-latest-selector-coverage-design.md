# CLI Process-Level Latest Selector Coverage Design

Date: 2026-04-01
Issue: `#791`
Status: Proposed for the current task branch

## Problem

The repository already proves three lower layers of `--session latest`:

1. repository selection returns the newest resumable root session
2. app-layer CLI runtime bootstrap resolves `latest` into a concrete session id
3. daemon CLI parsing accepts the literal `latest` token for `ask` and `chat`

What is still missing is a true spawned-process proof that the `loong` binary consumes that
resolved session id correctly in operator-visible flows.

That gap matters because the current tests would still stay green if:

1. spawned CLI argument wiring bypassed the selector-aware runtime path
2. chat startup or history commands rendered the wrong session after bootstrap
3. ask loaded the wrong conversation history before issuing the provider request

## Goal

Add the smallest stable process-level daemon integration slice that proves:

1. `loong chat --session latest` surfaces the resolved root session in startup output
2. `loong chat --session latest` loads history from the selected latest resumable root session
3. `loong chat --session latest` fails clearly when no resumable root session exists
4. `loong ask --session latest` sends provider traffic using the selected latest resumable root
   session context
5. `loong ask --session latest` fails clearly when no resumable root session exists

## Non-Goals

1. no selector semantics change
2. no new selector DSL
3. no provider runtime refactor
4. no generic daemon integration harness abstraction unless the tests prove it is required
5. no sleep-driven timing logic inside the shared provider harness; any elapsed-time regression proof
   must stay test-local and explicitly justified

## Approaches Considered

### A. Keep all coverage below the process boundary

Pros:

1. cheapest change
2. reuses the existing app-layer coverage directly

Cons:

1. leaves the exact issue gap open
2. does not prove real binary wiring or operator-visible output

### B. Add spawned-process tests with minimal file-local fixtures

Pros:

1. proves the real `loong` binary path
2. keeps changes local to daemon integration tests
3. reuses existing sqlite session semantics instead of inventing new fakes
4. can stay deterministic by using seeded sqlite state and a local mock provider

Cons:

1. requires some fixture setup for config, sqlite seeding, and request capture

### C. Build a generic shared CLI process test framework first

Pros:

1. could reduce duplication across future CLI process tests

Cons:

1. adds abstraction before the real need is proven
2. increases change surface for a narrowly scoped issue

## Decision

Choose approach B.

The smallest correct move is:

1. extend `crates/daemon/tests/integration/chat_cli.rs` with sqlite-backed `latest` fixtures and
   spawned `loong chat` coverage
2. add a new sibling integration file for spawned `loong ask` coverage because no existing ask
   process suite exists
3. use a local mock provider server for `ask` so the test can assert the selected history reached
   the provider request body without depending on external credentials or network access

This keeps the scope fully inside daemon integration tests, preserves the existing app/runtime
ownership boundaries, and avoids broad new abstractions.

## Test Design

### Chat process path

Seed sqlite with:

1. one older resumable root session
2. one newer resumable root session
3. one newer delegate-child distractor
4. one newest archived root distractor

Run:

```bash
loong chat --config <fixture> --session latest
```

Pipe scripted stdin:

```text
/history 8
/exit
```

Assert:

1. startup output shows `session: <newest-root>`
2. history output contains only the newest resumable root turns
3. distractor session content does not appear

Add a second test with no eligible root session and assert the process exits non-zero with a clear
`latest` selector error.

### Ask process path

Seed sqlite with the same style of session set.

Configure a local mock provider endpoint that records the incoming request and returns a trivial
successful response.

Run:

```bash
loong ask --config <fixture> --session latest --message "Summarize the current session."
```

Assert:

1. process exits successfully
2. captured provider request body contains the selected latest root session turns
3. captured provider request body does not contain distractor session turns

Add a second test with no eligible root session and assert the process exits non-zero with a clear
`latest` selector error before any provider request is issued.

Add one regression test with a bounded setup delay that exceeds the old fixed mock-server budget so
the suite proves the wait window starts with the spawned process run rather than server creation.

## Validation Strategy

Minimum required validation for this slice:

1. write the new spawned-process tests first and watch them fail
2. implement only the minimal fixture support needed to make them pass
3. run focused daemon integration tests for the new process-level coverage
4. rerun broader daemon and workspace verification before claiming readiness
