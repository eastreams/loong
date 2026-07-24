# Open Architecture Questions

This file records decisions that the rewrite deliberately does not settle and
temporary conclusions that constrain those questions. Code should not introduce
placeholder owners or compatibility abstractions to make an open question
appear resolved.

## Actix Actor Model (Temporary Decision)

Loong temporarily adopts Actix's actor ownership and polling model. This fixes
the architecture vocabulary for further design; it does not yet make the Actix
crate or the types below a stable public dependency.

```rust
type RuntimeHandle = actix::Addr<RuntimeActor>;

struct AgentHost<A> {
    agent: A,
    runtime: RuntimeHandle,
    allowed: AllowedCapabilities,
}

pub struct Context<'a, A>
where
    A: actix::Actor<Context = actix::Context<A>>,
{
    actor: &'a mut actix::Context<A>,
    runtime: &'a RuntimeHandle,
    allowed: &'a AllowedCapabilities,
}
```

The final application owns the Actix system lifecycle and the root
`RuntimeHandle`. `RuntimeActor` is the sole mutable owner of runtime state, and
each `AgentHost<A>` is an Actix actor that owns one business agent plus its
runtime address and current capability ceiling; the business `A` is not a
second actor. A Loong `Context<'_, AgentHost<A>>` is constructed privately for
each handler invocation or actor-future poll; callers cannot replace its
address, forge its allowed capabilities, or obtain the raw runtime address
through its public API.

Asynchronous handlers that need actor state use `ResponseActFuture`. Actix lends
fresh `&mut AgentHost<A>` and `&mut actix::Context<_>` references on each poll and
releases them on `Pending`, so the mailbox may process another message while the
response is waiting. `AtomicResponse` or `ctx.wait` is the explicit boundary for
work that must freeze mailbox processing. Protected admission must use bounded
`Addr::send`/`try_send`; `do_send` must not bypass capacity on those paths.

This runtime context proves a valid actor execution environment, not permission
to perform a side effect. Tools still receive only narrowed access facades, and
side-effect code still requires `Granted<ConcreteAction>`. Nested capability
ceilings may only shrink; that does not create an implicit parent-child lifetime.

The [runtime model review](../RUNTIME-MODEL-REVIEW.md) records the Actix source
comparison. Its [Tokio-only probe](../prototypes/runtime-model) validates a
stricter serialized alternative and its cancellation invariants; that ordinary
borrowing-handler scheduler is not part of this temporary Actix conclusion.

## Session, Turn, and Step Semantics

The actor model settles state ownership and poll-time scheduling. An actor,
message, or response future is not automatically a product `Session`, `Turn`,
or `Step`.

Still unresolved:

- What owns a session, how does it relate to root agents, and when does it end?
- What external input/output API drives a session and exposes streamed events?
- What event begins and commits a turn, and which state survives it?
- Are steps policy/audit units, scheduling units, or both?
- How do request abandonment, explicit cancellation, actor stop, panic, and
  partial side effects map to product-visible outcomes and audit records?
- Which active response futures are cancelled during shutdown or session
  cancellation? Dropping an Actix request does not by itself guarantee that an
  already-started response future stops.
- Which actor failures are terminal, supervised, or restartable?

## Execution Plane Ownership

- Is an execution plane selected by the kernel, an access implementation, or
  another composition owner?
- Is plane selection policy-visible metadata or internal routing?
- How can routing remain auditable without embedding a plane into `Action`?

`ExecutionPlane` must not be added to `Action` until this is decided.

## Parent Policy Request Semantics

The application context locates the parent boundary by implementing
`ParentGrantRequester`. `RequiresApproval` passes the same concrete action to
that boundary and returns the parent's final `Granted<A>` or denial; the child
does not expand its own capability ceiling or mint a replacement grant.

Still unresolved:

- Does dropping a `ParentGrantRequester` future cancel parent evaluation, or
  only stop waiting for its result?
- Is a separate approval-request audit event required, and how does it refer to
  the final `GrantId` without becoming another authorization proof?

No `SessionAuthority`, `CapabilityToken`, or second grant proof should be
introduced to answer these questions.

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

Until measurement exists, high performance is a design goal rather than a
quantified compatibility promise.
