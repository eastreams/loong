# Runtime-Self Advisory Boundary Design

Date: 2026-03-24
Epic: #440
Related PR overlap to respect: #464
Status: approved for implementation

## Goal

Harden the runtime boundary between authoritative self lanes and advisory memory
lanes so LoongClaw no longer relies mainly on explanatory prose to prevent
advisory memory from looking like identity authority.

## Current State

LoongClaw now has explicit runtime-self and resolved-identity lanes.

Those lanes are already stronger than the earlier `profile_note`-only shape.

The remaining gap is at prompt projection time.

`Session Profile`, `Memory Summary`, and `RetrievedMemory` are still projected as
plain `system` messages in both provider-direct and default-context-engine
assembly.

That means the effective authority boundary still depends too heavily on the
text inside those messages.

The weakest cases are:

- advisory content can contain runtime-owned headings such as
  `## Resolved Runtime Identity`
- advisory profile text can contain identity-shaped headings such as
  `# Identity`
- provider-direct and kernel-bound assembly each perform their own advisory
  projection match, which invites drift

## Non-Goals

- do not redesign the overall prompt topology
- do not move session profile into the base runtime-self system prompt
- do not replace or supersede `#464` staged hydration work
- do not add a new memory store or identity store
- do not let `soul_guidance` become an identity resolution source

## Options Considered

### Option 1: documentation and tests only

Keep the current projection shape.

Add stronger docs and regression tests.

This is the smallest patch, but it leaves the root cause untouched because
authority is still enforced mostly by wording.

### Option 2: typed advisory projection governance

Introduce one shared advisory projection seam for prompt assembly.

Keep the current prompt topology.

Sanitize advisory content when it is projected into prompt space.

Demote runtime-owned or identity-like headings inside advisory content so they
cannot masquerade as authoritative sections.

Use the same projection rule in provider-direct and kernel-bound assembly.

This is the recommended option because it fixes the root cause with the minimum
architectural change.

### Option 3: structural prompt rewrite

Move advisory profile and durable recall out of memory hydration and into a new
separate prompt topology.

This would create the strongest boundary, but it would overlap too heavily with
`#464` and would turn a governance fix into a larger prompt architecture
rewrite.

## Chosen Design

Use option 2.

Add a small shared advisory prompt governance module.

That module will own the rule for demoting runtime-owned or identity-like
headings that appear inside advisory content.

The rule will be intentionally narrow:

- preserve normal advisory text
- preserve current top-level advisory containers such as
  `## Session Profile`,
  `## Memory Summary`, and
  `## Advisory Durable Recall`
- demote nested headings that look like authoritative runtime sections or
  identity-shaped headings

Examples of headings that should be demoted inside advisory content:

- `## Runtime Self Context`
- `### Standing Instructions`
- `### Tool Usage Policy`
- `### Soul Guidance`
- `### User Context`
- `## Resolved Runtime Identity`
- `## Session Profile`
- `# Identity`
- `## Imported IDENTITY.md`
- `## Imported IDENTITY.json`

Demotion means the heading stays visible as advisory reference text, but it no
longer renders as a prompt section heading.

## Scope of Code Changes

### 1. Advisory content sanitization

Create a small helper that rewrites only governed heading lines.

The helper should be deterministic and string-based.

It should not attempt semantic parsing.

### 2. Session profile rendering

Update `runtime_identity::render_session_profile_section(...)` to sanitize the
advisory profile body before it is wrapped in the `## Session Profile` section.

This ensures the same protection applies to:

- live profile-note hydration
- stored runtime-self continuity profile projection

### 3. Shared prompt projection for memory entries

Replace the duplicated advisory-entry projection logic in:

- `provider/request_message_runtime.rs`
- `conversation/context_engine.rs`

with one shared helper.

That helper should:

- keep `Turn` entries on the history path
- sanitize advisory entries before turning them into prompt messages

### 4. Regression coverage

Add tests that prove:

- identity-like headings in advisory profile content are demoted
- runtime-owned headings inside durable recall are demoted
- live identity still wins over stored or advisory content
- direct and kernel-bound prompt assembly keep the same advisory projection for
  profile entries
- `soul_guidance` still cannot produce `Resolved Runtime Identity`

## Why This Is Minimal and Correct

This design does not create a new prompt lane.

This design does not expand the runtime identity resolver.

This design does not depend on hardcoded file names beyond the runtime-owned
section names that LoongClaw already owns.

This design keeps the current architecture intact while making the actual
authority boundary explicit in code instead of leaving it mostly implicit in
message prose.

## Interaction With #464

`#464` changes staged memory hydration and artifact transport.

This design stays above that layer.

The new governance helper will operate on projected advisory content, so staged
or unstaged hydration can both reuse it later.

That keeps this slice mergeable before or after `#464`.

## Validation Plan

- write failing tests first
- run targeted red tests and confirm the right failures
- implement the smallest projection-governance helper that makes them pass
- run targeted green tests
- run formatting and clippy on the touched surface
- run full workspace tests

## Expected Outcome

After this change:

- LoongClaw still has the same high-level runtime-self architecture
- advisory memory can no longer regain authority by replaying runtime-owned
  headings
- provider-direct and kernel-bound prompt assembly will use one advisory
  projection rule
- future staged hydration work can inherit the same governance rule instead of
  re-implementing it
