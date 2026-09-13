# Open Architecture Questions

This file records decisions that the rewrite deliberately does not settle and
temporary conclusions that constrain those questions. Code should not introduce
placeholder owners or compatibility abstractions to make an open question
appear resolved.

## Implemented Actor Runtime, Open Product Ownership

[`loac`](https://docs.rs/loac/0.2.0/loac/) is the source of truth for actor
semantics. It owns actor state, bounded mailbox admission, reply scheduling, and
subtree lifecycle on Tokio. Those runtime responsibilities do not by themselves
define a product `Session`, `Turn`, or `Step`.

[`loong-kernel`](../crates/kernel/src/lib.rs) has migrated its policy actor to
`loac`. The owner of each root actor tree, and how that ownership relates to
product session lifecycle, remain open; do not introduce a compatibility
facade to hide those gaps.

Actor execution context is not side-effect authority. Tools still receive only
narrowed access APIs, and side-effect code still requires
`Granted<ConcreteAction>`.

## Session, Turn, and Step Semantics

`loac` settles actor-local ownership, scheduling, and subtree lifecycle,
but not product ownership. An actor, message, or reply is not automatically a
product `Session`, `Turn`, or `Step`.

Still unresolved:

- What owns a session, how does it relate to root agents, and when does it end?
- What external input/output API drives a session and exposes streamed events?
- What event begins and commits a turn, and which state survives it?
- Are steps policy/audit units, scheduling units, or both?
- How do request abandonment, explicit cancellation, actor shutdown, panic, and
  partial side effects map to product-visible outcomes and audit records?
- Which Stop, Drain, or Kill transition implements each product cancellation
  point? Dropping a call or response future is not a session cancellation
  protocol: before admission no message commits; after admission but before
  dispatch it permits the runtime to skip the handler; after dispatch it
  abandons only the result without rolling back handler effects or cancelling
  the selected reply.
- Which actor failures are terminal, supervised, or restartable?

## Execution Plane Ownership

- Is an execution plane selected by the kernel, an access implementation, or
  another composition owner?
- Is plane selection policy-visible metadata or internal routing?
- How can routing remain auditable without embedding a plane into `Action`?

`ExecutionPlane` must not be added to `Action` until this is decided.

## Parent Policy Request Semantics

Parent approval is not implemented yet. `RequiresApproval` currently maps to
denial in the kernel engine; no `ParentGrantRequester` type locates the parent
boundary, and the child does not expand its own capability ceiling or mint a
replacement grant.

Still unresolved:

- Does dropping a `ParentGrantRequester` future cancel parent evaluation, or
  only stop waiting for its result?
- Is a separate approval-request audit event required, and how does it refer to
  the final `GrantId` without becoming another authorization proof?

No `SessionAuthority`, `CapabilityToken`, or second grant proof should be
introduced to answer these questions.

## Stream Reply Mode

`loac` replies once per message. Streaming is an application composition: the
subscriber owns a channel receiver and passes the sender as message data; the
subscribe handler returns a future whose owned task produces items into that
channel, so the actor stores no subscriber state. The
[streaming example](https://github.com/InuDial/loac/blob/loac-v0.2.0/crates/loac/examples/dispatch/streaming.rs)
documents this shape.

A runtime-driven stream reply — a handler returning a stream that the runtime
polls per item with actor borrows — is deliberately not implemented. Still
unresolved:

- Who polls each item, and does each poll receive actor state?
- Which scheduling slot or capability bounds each active stream?
- What channel capacity and backpressure policy connects runtime and caller?
- When does `call` resolve, and how do Stop, Drain, and Kill close active
  streams?
- Does dropping the caller's stream cancel the pump and release its slot?

Do not add a stream reply strategy until a product feature requires per-item
actor state without mailbox turns.

## Runtime Extension Trust Boundary

The product direction includes both compile-time Rust extensions and runtime
extensions such as scripts. Before implementing the latter, decide:

- Which language or engine is supported?
- Are scripts trusted, language-isolated, process-sandboxed, or configurable?
- Which narrow host APIs are visible, and can a script reach the OS directly?
- How are script requests converted into concrete actions without exposing the
  kernel, runtime, backend, or global context?
- How are extension identity, versioning, loading, and revocation represented?

An unrestricted in-process script cannot be called safe merely because the
host also has policy types.

## Performance Contract

- Which representative workloads define high performance for Loong?
- Which latency, throughput, allocation, binary-size, and memory metrics matter?
- Which regressions should become enforced benchmark gates?

Actor benchmarks now quantify selected runtime paths, but representative product
workloads, accepted baselines, and enforced regression gates remain open. Until
those are defined, high performance is not a quantified compatibility promise.
