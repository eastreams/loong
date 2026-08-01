# Loong Architecture

Loong keeps crate dependencies one-way and sends protected side effects through
one authorization path. Detailed API rules live in the module docs linked
below.

## Crate Relationships

Arrows point from a crate to its dependency:

```text
loong-cli -> loong-kernel -> loong-access
loong-access -> loong-core -> loong-contracts
loong-access -> loong-contracts
```

`loong-contracts` defines data shared between layers. `loong-core` builds the
authorization model on those contracts. `loong-access` uses both to expose
domain operations. `loong-kernel` composes access implementations, and final
application assembly belongs in `loong-cli`. A lower crate never depends on a
crate above it.

## Access -> Action -> Policy

```text
caller or tool
    -> Access
    -> concrete Action
    -> Policy
    -> Granted<Action>
    -> Action::run or backend
```

An [access API](crates/access/src/lib.rs) is the only operation surface given to
a caller. It exposes a narrow set of requests and does not reveal the kernel,
session, runtime, backend, or a global context.

Tool and access boundaries differ in representation. Runtime tool discovery
and dispatch erase the concrete tool implementation type. Access does not erase
its action type: the concrete `A` remains intact through policy, grant, and
execution so the backend receives `Granted<ConcreteAction>`.

Each request becomes a [concrete action](crates/core/src/action.rs).
`ActionMeta` gives policy the action's name, payload, and required capabilities;
generic policy may inspect it through a borrowed `dyn ActionMeta` view, but that
does not erase the owned action. Core still owns the concrete `A`, so the
resulting proof remains `Granted<A>`. An action describes what should happen,
not where it runs.

[Policy](crates/core/src/policy.rs) evaluates that same action but does not
execute it. The capabilities allowed by the
[policy context](crates/core/src/policy/context.rs) are a ceiling. If an action
needs more, a parent may evaluate the same action; the current context cannot
expand its own authority.

After policy allows the action and the decision is recorded, the
[policy engine](crates/core/src/policy/engine.rs) may create `Granted<A>`.
`Granted<A>` binds the recorded `GrantId` to the concrete action and is the only
proof accepted by side-effect code. If `A` implements `Action<Cx>`,
`Granted<A>::run` consumes the grant and calls `Action::run`. Otherwise, a
backend may consume `Granted<ConcreteAction>` directly. A backend must not
accept a raw action and repeat the permission check itself.

`GrantId` only identifies the grant record. The concrete policy implementation
assigns its opaque UUID; callers have no ordering contract. It does not
authorize execution, and it is not a second proof.

## Runtime Actor Model

[`loong-actor`](crates/actor/src/lib.rs) implements the Tokio actor contract:
bounded mailbox admission, an independent `max_in_flight` limit, separate
`ActorRef` and `ActorOwner` roles, actor-owned child lifecycles,
ready/owned/interleaved/exclusive reply scheduling, and Stop/Drain/Kill
termination. This settles actor-local ownership and scheduling.

[`loong-kernel`](crates/kernel/src/lib.rs) still contains an Actix prototype that
has not migrated to this runtime. It is not a second supported actor model. The
owner of each root actor tree, and how that ownership relates to product session
lifecycle, remain open questions; migration must not hide those gaps behind a
compatibility facade. See [Open Architecture Questions](docs/open-questions.md).

## Open Questions

Session, turn and step semantics, active cancellation and supervision,
execution-plane ownership, approval audit, and runtime extension isolation
remain unresolved. See [Open Architecture Questions](docs/open-questions.md).
