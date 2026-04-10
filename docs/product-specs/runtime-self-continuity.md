# Runtime-Self Continuity

## User Story

As a LoongClaw operator, I want runtime-self continuity to survive compaction,
delegation, and future durable recall so that the agent stays coherent without
mixing identity authority with transient task context.

## Continuity Lanes

- `Runtime Self Context`
  Standing instructions and soul guidance loaded from runtime-self sources such
  as `AGENTS.md`, `CLAUDE.md`, and `SOUL.md`. This lane remains runtime
  guidance. It is reloaded through the normal runtime path instead of being
  copied into durable memory.

- `Resolved Runtime Identity`
  The identity authority for the active session chain. It is resolved from
  runtime-self sources first and can fall back to legacy imported identity from
  `profile_note`. Compaction, session profile projection, and future retrieval
  must not override this lane.

- `Session Profile`
  Durable advisory context such as preferences, tuning, and imported
  non-identity profile material. Future durable recall from `#421` and `#429`
  may enrich this lane, but that enrichment remains advisory and cannot become a
  second identity authority.

- `Session-Local Recall`
  Memory summaries, sliding-window turns, and delegate child task findings.
  These artifacts preserve useful session context, but they stay local to the
  session chain unless a separate durable-memory flow explicitly promotes them.

## Boundary Rules

- Compaction and summarization preserve continuity by treating summary blocks as
  session-local recall only.
- Delegate child sessions inherit continuity through one explicit runtime
  contract, even when the child has no extra tool narrowing.
- Durable recall augments advisory context; it does not replace runtime-self
  guidance or resolved runtime identity.
- Advisory profile, summary, and durable-recall projection must demote
  runtime-owned or identity-like headings instead of replaying them as
  authoritative-looking prompt sections.
- When a safe workspace file root is configured and compaction is about to run,
  LoongClaw may export advisory durable recall into `memory/YYYY-MM-DD.md`
  before compaction proceeds.
- When a safe workspace file root is configured and durable memory files are
  present, LoongClaw may bootstrap advisory durable recall from `MEMORY.md`,
  `memory/MEMORY.md`, and recent `memory/YYYY-MM-DD.md` logs into runtime
  context. Workspace memory frontmatter may further mark records as active,
  superseded, tombstoned, or archived; inactive records must not be replayed.
- Session-local content is never promoted into durable self-state implicitly.

## Selected Direction

LoongClaw's public continuity direction is intentionally narrow:

- keep runtime-self guidance and resolved runtime identity as the authority lane
- keep session profile and future durable recall advisory
- add bounded durable context before broadening retrieval scope
- add explicit query-aware retrieval before embedding-dependent search becomes
  required

In practical terms, that means:

- durable hot memory should stay bounded and operator-legible instead of turning
  into a second identity layer
- retrieval should start with local text-first recall and clear provenance
  rather than jumping directly to opaque vendor-managed search
- future memory providers may enrich advisory context, but they must not become
  prompt authority or override runtime-owned continuity lanes

Related public specs:

- [Memory Profiles](memory-profiles.md) covers bounded profile projection and
  advisory durable context
- [Memory Retrieval](memory-retrieval.md) covers explicit query-aware recall and
  provenance expectations

## Acceptance Criteria

- Summary blocks clearly state that they do not replace runtime-self guidance,
  resolved runtime identity, or durable advisory profile context.
- Session profile projection clearly states that durable recall is advisory and
  does not override resolved runtime identity.
- Advisory profile, summary, and durable-recall projection demote runtime-owned
  or identity-like headings rather than preserving them as raw prompt section
  headings.
- Delegate child sessions always receive an explicit self-continuity contract.
- Pre-compaction durable exports, when enabled by workspace configuration, stay
  advisory and do not become an identity override path.
- Runtime durable-recall bootstrap, when enabled by workspace configuration,
  stays advisory and does not become an identity override path.
- Session-local derived overview artifacts stay advisory and are clearly
  distinguished from summary checkpoints and retrieved durable recall.
- The relationship to `#421` and `#429` is explicit: retrieval may enrich
  durable context, but it must not become an identity override path.
