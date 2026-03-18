# Tool Search Follow-up Payload Compactor Design

**Problem**

`tool.search` returns a structured payload that can include up to eight results, and each result currently carries lease-bearing discovery data plus search-only explanation fields such as `why` and `tags`. In the current `alpha-test` branch, discovery-first follow-up, turn-loop follow-up, and repeated-tool-guard replay forward that payload back into the next model round largely unchanged unless generic character-budget truncation fires.

This is a model-context efficiency problem. The follow-up model needs enough information to choose and invoke a discovered tool, but it does not need the full raw discovery payload shape in every round.

**Critical Constraint**

Unlike `file.read`, `tool.search` follow-up payloads cannot be safely handled by simply setting `payload_truncated=true`.

`provider/shape.rs` deliberately skips truncated `tool.search` envelopes when reconstructing discovery bridge context because that bridge trusts only intact discovery results when extracting `tool_id -> lease` bindings for later `tool.invoke` calls.

That means this slice must preserve bridge-safe semantics while still shrinking the payload.

**Constraints**

- Do not change `tool.search` execution output in `crates/app/src/tools/mod.rs`.
- Do not change `TurnEngine` tool-result envelope generation.
- Do not add a new runtime config knob.
- Do not widen this into a generic all-tool reducer framework.
- Do not break `provider/shape.rs` lease extraction for discovery follow-up.
- Do not change `external_skills.invoke` follow-up semantics.

**Approaches Considered**

1. Mark reduced `tool.search` follow-up payloads as truncated.
   Rejected because `provider/shape.rs` intentionally ignores truncated discovery results, which would break later `tool.invoke` lease reconstruction.

2. Relax provider bridge parsing so it accepts truncated `tool.search` payloads.
   Rejected because it changes trust semantics in the bridge layer and broadens the blast radius beyond a performance-focused follow-up slice.

3. Apply a follow-up-only structural compactor that preserves bridge-required fields and avoids setting `payload_truncated=true`.
   Recommended because it keeps execution semantics unchanged, preserves discovery bridge behavior, and reduces token waste by pruning low-value fields instead of changing trust semantics.

**Chosen Design**

Add a shared helper in `turn_shared.rs` that rewrites only follow-up `tool_result` lines whose envelope tool is `tool.search` and whose nested `payload_summary` is valid JSON.

The compactor will:

- parse the outer tool-result envelope
- parse the nested `payload_summary`
- preserve outer `payload_chars`
- preserve outer `payload_truncated` as `false`
- preserve every returned result entry so the model can still choose among all visible discovered tools
- preserve per-result fields required for follow-up action:
  - `tool_id`
  - `summary`
  - `argument_hint`
  - `required_fields` when non-empty
  - `required_field_groups` when non-empty
  - `lease`
- drop follow-up-only low-value fields:
  - top-level `adapter`
  - top-level `tool_name`
  - top-level `returned`
  - per-result `tags`
  - per-result `why`
- preserve top-level `query` because the provider-generated discovery query can differ from the original user wording and still helps explain why these results were surfaced
- leave non-`tool.search` payloads unchanged

**Why This Shape**

This is the smallest safe compaction that still preserves the information needed for:

- provider bridge lease reconstruction
- model-side tool selection
- model-side argument planning for `tool.invoke`

Keeping all returned results avoids a recall regression where the correct candidate is omitted from the follow-up context. Pruning per-result explanation metadata removes noise without changing the actionable semantics of discovery.

**Integration Points**

Run the compactor only in follow-up message assembly:

- discovery-first follow-up assembly in `turn_coordinator.rs`
- turn-loop tool-result follow-up assembly in `turn_loop.rs`
- repeated-tool-guard replay in `turn_loop.rs`

Do not apply it to raw tool output delivery.

**Testing Strategy**

Add focused coverage for:

- discovery-first follow-up compacting `tool.search` payload summaries
- turn-loop follow-up compacting `tool.search` payload summaries
- repeated-tool-guard replay compacting `tool.search` payload summaries
- provider bridge lease extraction continuing to work with compacted discovery follow-up payloads
- `external_skills.invoke` payload handling remaining unchanged

**Risk Assessment**

This is a low-risk slice because it only changes model-facing follow-up assembly and leaves tool execution plus outer envelope generation untouched.

The main residual risk is merge overlap with other open performance PRs that touch the same conversation follow-up files. That is a branch-management concern, not a runtime design concern.
