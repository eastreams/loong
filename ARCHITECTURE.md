# Loong Architecture

Loong keeps crate dependencies one-way and sends protected side effects through
one authorization path. Detailed API rules live in the module docs linked
below.

## Crate Relationships

Arrows point from a crate to its dependency:

```text
loong-cli -> loong-kernel -> loong-contracts
loong-kernel -> loac -> loac-macros
loong-agent -> loong-context -> loong-contracts
loong-agent -> loong-provider -> loac
loong-agent -> loac
```

`loong-contracts` defines data shared between layers. `loong-kernel` builds the
authorization model on those contracts and exposes domain access facades.
`loong-context` stores working-context transcripts, and `loong-provider` streams
upstream items through ordered failover; `loong-agent` is the application-side
skeleton that composes those services. `loac` is the actor runtime and
`loac-macros` its procedural-macro companion; kernel and agent use it for
actors, while provider only depends on its `Writer` contract. Final application
assembly belongs in `loong-cli`. A lower crate never depends on a crate above
it.

## Access -> Action -> Policy

```text
caller or tool
    -> Access
    -> concrete Action
    -> Policy
    -> Granted<Action>
    -> Action::run or backend
```

An [access API](crates/kernel/src/access.rs) is the only operation surface given to
a caller. It exposes a narrow set of requests and does not reveal the kernel,
session, runtime, backend, or a global context.

Tool and access boundaries differ in representation. Runtime tool discovery
and dispatch erase the concrete tool implementation type. Access does not erase
its action type: the concrete `A` remains intact through policy, grant, and
execution so the backend receives `Granted<ConcreteAction>`.

Each request becomes a [concrete action](crates/kernel/src/policy/action.rs).
`ActionMeta` gives policy the action's name, payload, and required capabilities;
generic policy may inspect it through a borrowed `dyn ActionMeta` view, but that
does not erase the owned action. The kernel still owns the concrete `A`, so the
resulting proof remains `Granted<A>`. An action describes what should happen,
not where it runs.

[Policy](crates/kernel/src/policy.rs) evaluates that same action but does not
execute it. The capabilities allowed by the
[policy context](crates/kernel/src/policy.rs) are a ceiling; the current context
cannot expand its own authority. Whether a parent may evaluate the same action
above that ceiling is still unresolved (see
[Open Architecture Questions](docs/open-questions.md)).

After policy allows the action and the decision is recorded, the
[policy engine](crates/kernel/src/policy/engine.rs) may create `Granted<A>`.
`Granted<A>` binds the recorded `GrantId` to the concrete action and is the only
proof accepted by side-effect code. If `A` implements `Action<Cx>`,
`Granted<A>::run` consumes the grant and calls `Action::run`. Otherwise, a
backend may consume `Granted<ConcreteAction>` directly. A backend must not
accept a raw action and repeat the permission check itself.

`GrantId` only identifies the grant record. The concrete policy implementation
assigns its opaque UUID; callers have no ordering contract. It does not
authorize execution, and it is not a second proof.

## Runtime Actor Model

[`loac`](https://docs.rs/loac/0.2.0/loac/) implements the Tokio actor contract:
bounded mailbox admission, an independent `max_in_flight` limit, separate
`ActorRef` and `ActorOwner` roles, actor-owned child lifecycles,
ready/owned/interleaved/exclusive reply scheduling, and Stop/Drain/Kill
termination. This settles actor-local ownership and scheduling.

[`loong-kernel`](crates/kernel/src/lib.rs) has migrated its policy actor to
`loac`. The owner of each root actor tree, and how that ownership relates to
product session lifecycle, remain open questions; do not reintroduce a
compatibility facade that hides those gaps. See
[Open Architecture Questions](docs/open-questions.md).

### Actor Handle Boundary

`ActorRef<A>` holds one typed `Arc<ActorInner<A>>`. The inner value contains
the mailbox sender and lifecycle control. `ActorOwner<A>` adds RAII ownership
without another allocation. `ChildSet` stores `Arc<dyn ErasedActor>`. That
pointer shares the typed inner allocation. Dynamic dispatch stays on cold
ownership paths.

Queued calls retain only `Weak<ActorInner<A>>`. A strong edge would make the
queue own its sender. That creates a cycle. Dispatch creates a strong typed
permit only after dequeue. `Mode` remains the lifecycle authority. A private
`Notify` only wakes the actor task.

## Open Questions

Session, turn and step semantics, active cancellation and supervision,
execution-plane ownership, approval audit, and runtime extension isolation
remain unresolved. See [Open Architecture Questions](docs/open-questions.md).
