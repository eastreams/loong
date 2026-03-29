# Reference Runtime Comparison

Date: 2026-03-27
Issue: #652
LoongClaw baseline reviewed for merge: `loongclaw-ai/loongclaw` `dev@fe87d347`

## Scope

This document compares LoongClaw against three reference runtimes that now put
the clearest product pressure on the next LoongClaw slices:

- OpenClaw
- NanoBot
- PicoClaw

The comparison is intentionally narrow. It focuses on:

- background task and async work UX
- skills discovery and first-use UX
- memory retrieval, scope, and provenance

It does not cover:

- Web UI
- channel breadth
- generic benchmark marketing claims
- full parity against any single reference runtime

## Why These References

OpenClaw, NanoBot, and PicoClaw each expose a more operator-visible product surface
than LoongClaw on at least one of the three scoped areas above.

- OpenClaw already productizes cron jobs, community skill discovery, and memory
  search as explicit operator surfaces.
- NanoBot frames itself as an always-on agent runtime with persistent memory,
  background execution concepts, and MCP-oriented expansion.
- PicoClaw moves quickly on compact user-facing features such as cron support,
  ClawHub-based skill discovery, and memory/runtime ergonomics, even when some
  roadmap items are still incomplete.

These references matter because they pressure LoongClaw at the product layer,
not because they are stronger at governance or runtime boundaries.

## Current LoongClaw Evidence

LoongClaw is stronger than older repo-local summaries sometimes imply. Current
`dev` already proves substantial substrate in all three scoped areas.

### Background Task Substrate Already Exists

LoongClaw already exposes session and delegate runtime tools that are more
governed than a typical "background task" MVP:

- `approval_requests_list`
- `approval_request_status`
- `approval_request_resolve`
- `delegate`
- `delegate_async`
- `session_status`
- `session_wait`
- `session_events`
- `session_cancel`
- `session_recover`
- `session_tool_policy_status`
- `session_tool_policy_set`
- `session_tool_policy_clear`

The important conclusion is that LoongClaw does not need a second async engine
to start closing the task UX gap. It already has child-session execution,
session visibility, approval, recovery, and session-scoped tool narrowing.

### External Skills Substrate Already Exists

LoongClaw already ships a managed external-skills runtime rather than only a
download primitive:

- `external_skills.fetch`
- `external_skills.install`
- `external_skills.list`
- `external_skills.inspect`
- `external_skills.invoke`
- `external_skills.remove`
- `external_skills.policy`

The runtime also already maintains a multi-scope discovery inventory with:

- managed installs
- user scope
- project scope
- eligibility filtering
- model visibility
- invocation policy
- shadowed-skill detection
- operator CLI support

This is not a weak substrate. The gap is not "how to install skills". The gap
is "how users discover the right skill and understand why it is or is not the
appropriate next step".

### Memory Substrate Already Exists

Current `dev` already has more memory structure than the quality summary line
in `docs/QUALITY_SCORE.md` suggests:

- `MemoryScope` is typed as `Session`, `User`, `Agent`, and `Workspace`
- canonical memory records distinguish typed memory kinds
- the stage vocabulary already includes `Derive`, `Retrieve`, `Rank`,
  `AfterTurn`, and `Compact`
- memory retrieval requests are already modeled explicitly
- runtime-self continuity boundaries already separate:
  - `runtime self context`
  - `resolved runtime identity`
  - `session profile`
  - `session-local recall`

The gap is that the shipped built-in retrieval path is still intentionally
narrow:

- `query` is `None`
- scopes are `[Session]`
- allowed kinds are `[Summary]`

So LoongClaw already has a more explicitly structured memory architecture in
these documented areas, but it still does not ship an operator-visible
retrieval product.

## Comparative Snapshot

| Surface | OpenClaw | NanoBot | PicoClaw | LoongClaw | Main gap |
| --- | --- | --- | --- | --- | --- |
| Background tasks | Productized cron and task delivery surfaces | Persistent runtime story with cron and heartbeat concepts surfaced in the repo | Cron tool shipped and iterated in changelog | Governed child-session substrate already exists | Missing task-shaped operator UX on top of session runtime |
| Skills discovery | ClawHub search/install/update already productized | Public positioning emphasizes agent/tool expansion; repo surfaces skill and MCP growth | ClawHub-based skill discovery shipped; `find_skill` still remains roadmap work | Managed runtime lifecycle, discovery inventory, and policy already exist | Missing discovery-first search, recommendation, and first-use flow |
| Memory retrieval | Operator-visible memory status/index/search surface | Persistent memory story is present, but provenance model is less explicit in public repo evidence | Practical memory/runtime iterations ship quickly, but provenance semantics are lighter | Typed scopes, staged retrieval vocabulary, and identity boundaries are already explicit | Missing query-aware scoped retrieval and user-visible provenance/search surface |
| Governance posture | Stronger than many agents, but more product-led than LoongClaw | Product-led runtime posture | Fast-moving product posture | Most explicit governance and boundary modeling in the available public docs in this comparison set | Productization lags the substrate |

## Surface Analysis

### 1. Background Tasks

OpenClaw sets the clearest current bar. Its cron surface already treats
background work as a first-class operator story with explicit lifecycle,
targets, and delivery semantics. NanoBot and PicoClaw also expose background
execution concepts in user-facing terms rather than raw runtime primitives.

LoongClaw already has the harder low-level pieces:

- background child-session execution through `delegate_async`
- visibility and lifecycle inspection through `session_*`
- explicit approval state
- per-session tool narrowing
- recovery paths

What it does not yet have is the product translation layer. Today the user is
still asked to reason in terms of:

- child sessions
- session ids
- visible session scope
- session policy mutation

That is runtime truth, but it is not yet task UX.

**Conclusion:** the next slice should productize existing session runtime into a
task-shaped operator surface before adding a new scheduler or cron runtime.

### 2. Skills Discovery

OpenClaw and PicoClaw currently win on discovery-first UX. OpenClaw's ClawHub
surface is already a user-facing registry. PicoClaw has shipped ClawHub-based
skill discovery and still tracks further `find_skill` work on its roadmap.
NanoBot's public positioning also leans toward runtime expansion as a normal
operator behavior.

LoongClaw's current managed runtime is already architecturally healthier than a
quick marketplace patch:

- install and remove are governed
- discovery is scope-aware
- eligibility and visibility are explicit
- invocation stays instruction-package oriented rather than pretending skills
  are native dynamic tools

But the user still needs to know too much before the flow starts. The current
surface is lifecycle-first:

- list
- inspect
- install
- invoke

The missing part is discovery-first:

- search
- recommend
- explain blocked or shadowed candidates
- provide a first task recipe after install

**Conclusion:** the next slice should reuse `SkillDiscoveryInventory` and add a
search-and-recommend layer instead of redesigning the skill runtime.

### 3. Memory Retrieval

OpenClaw's advantage is not only that it has memory search, but that the search
is operator-visible and explainable as a product surface. PicoClaw's advantage
is delivery velocity on practical memory ergonomics. NanoBot's public runtime
story also treats memory as a core product pillar.

LoongClaw, however, has an explicitly structured internal architecture
foundation for doing memory safely in these scoped areas:

- one canonical authority
- typed scope model
- staged retrieval vocabulary
- explicit fail-open posture
- explicit identity and runtime-self boundaries

That means LoongClaw should not start this convergence path by handing control
to an external memory vendor or by skipping directly to embeddings as the first
user-visible step.

The first gap to close is simpler and more important:

- make retrieval query-aware
- make scope selection explicit
- make provenance visible
- add local text-search fallback before optional embedding retrieval

**Conclusion:** LoongClaw should treat provenance-rich scoped retrieval as the
first product milestone, then layer semantic retrieval on top later.

## Cross-Cutting Conclusion

LoongClaw's next product gap is not "missing substrate". It is "missing product
translation".

Across all three scoped areas, the same pattern repeats:

1. the runtime already has meaningful internal capability
2. users still see low-level substrate concepts rather than product concepts
3. the missing work is default workflow design, not another foundational
   rewrite

That leads to one durable recommendation:

> LoongClaw should converge by productizing its current substrate, not by
> copying reference-product surfaces in parallel.

In practice that means:

- task UX should be built on session runtime
- skills discovery should be built on the current managed runtime inventory
- memory retrieval should be built on canonical records and staged retrieval

## Recommended Implementation Order

### First Slice: Background Task Productization

This should land first because it gives LoongClaw the clearest jump from
"governed runtime substrate" to "daily-usable agent runtime".

The first slice should:

- introduce a task-shaped operator contract over child sessions
- expose create, inspect, wait, follow, cancel, and recover in task language
- surface approval-pending and tool-policy state as task diagnostics
- keep cron and service-style scheduling out of scope for the first pass

### Second Slice: Skills Discovery-First UX

This should land second because it offers the highest UX lift per unit of new
runtime complexity.

The first slice should:

- add search/recommend on top of the existing discovery inventory
- explain eligibility, visibility, and shadowing
- return first-use guidance after install or inspect
- keep auto-install and dynamic per-skill tool registration out of scope

### Third Slice: Scoped Memory Retrieval with Provenance

This should land third because it touches the strongest architectural
boundaries and should not be rushed.

The first slice should:

- add query-aware retrieval requests
- broaden retrieval beyond session summary only
- surface provenance and injection reason
- ship local text search before embedding-dependent retrieval

## Non-Goals

This convergence path should not:

- introduce a parallel task runtime beside session runtime
- weaken approval, policy, or audit boundaries for convenience
- turn external skills into uncontrolled dynamic function tools
- make durable recall a second identity authority
- widen this track into Web UI work

## Related Artifacts

- Product specs:
  - `docs/product-specs/background-tasks.md`
  - `docs/product-specs/skills-discovery.md`
  - `docs/product-specs/memory-retrieval.md`
- Implementation plan:
  - `docs/plans/2026-03-27-runtime-productization-convergence-implementation-plan.md`
- Related issues:
  - `#217`
  - `#283`
  - `#292`
  - `#421`
  - `#652`

## External References

- OpenClaw cron jobs:
  `https://docs.openclaw.ai/automation/cron-jobs`
- OpenClaw ClawHub:
  `https://docs.openclaw.ai/tools/clawhub`
- OpenClaw memory CLI:
  `https://docs.openclaw.ai/cli/memory`
- NanoBot repository:
  `https://github.com/HKUDS/nanobot`
- PicoClaw changelog:
  `https://docs.picoclaw.io/docs/changelog/`
- PicoClaw roadmap:
  `https://docs.picoclaw.io/docs/roadmap/`

## Repo Evidence

- `crates/app/src/tools/catalog.rs`
- `crates/app/src/tools/session.rs`
- `crates/app/src/tools/external_skills.rs`
- `crates/daemon/src/skills_cli.rs`
- `crates/app/src/memory/canonical.rs`
- `crates/app/src/memory/stage.rs`
- `crates/app/src/memory/orchestrator.rs`
- `docs/product-specs/runtime-self-continuity.md`
- `docs/product-specs/memory-profiles.md`
