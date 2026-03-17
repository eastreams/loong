# File Read Follow-up Payload Reducer Design

**Problem**

`file.read` returns a structured payload with `path`, `bytes`, `truncated`, and `content`. In the current `alpha-test` branch, discovery-first follow-up, turn-loop follow-up, and repeated-tool-guard replay all forward that tool-result text back into the next model round with only generic character-budget truncation. When `content` is large but still below the primary tool-result summary limit, the model receives far more file text than it usually needs for the follow-up answer.

This is a token-cost and latency problem, not an execution correctness problem. The raw tool output still needs to stay intact when the user explicitly requests raw output.

**Constraints**

- Do not change `file.read` execution output in `crates/app/src/tools/file.rs`.
- Do not move reduction into `TurnEngine`; that would break raw-output semantics.
- Do not add new config knobs in this slice.
- Do not broaden this into a generic reducer framework for all tools.
- Preserve existing `tool.search` and `external_skills.invoke` follow-up semantics.

**Approaches Considered**

1. Reduce `file.read` in `TurnEngine`.
   Rejected because raw tool output requests should still receive the original tool result envelope.

2. Add a generic all-tool follow-up reducer.
   Rejected because only `file.read` is in scope here, and generalizing first would create abstraction debt without enough evidence.

3. Add a `file.read`-specific reducer only in follow-up message assembly.
   Recommended because it matches the real hotspot, preserves execution semantics, and reuses the existing follow-up mapping seam with minimal change.

**Chosen Design**

Add a shared helper in `turn_shared.rs` that rewrites only follow-up `tool_result` lines whose envelope tool is `file.read` and whose nested `payload_summary` is still valid JSON.

The reducer will:

- parse the outer tool-result envelope
- parse the nested `payload_summary`
- preserve `path`, `bytes`, and the original file-tool `truncated` flag
- replace large `content` with:
  - `content_preview`
  - `content_chars`
  - `content_truncated`
- set outer `payload_truncated=true` when follow-up reduction happens
- preserve outer `payload_chars`
- leave non-`file.read` results unchanged

This reducer will run in three places:

- discovery-first follow-up assembly
- turn-loop tool-result follow-up assembly
- repeated-tool-guard replay of the latest tool result

**Preview Strategy**

Use a head-only character preview, not head-tail or line-window compaction.

Reasons:

- smallest implementation surface
- consistent with existing generic truncation behavior
- easier to reason about and test
- enough for the model to understand the file type and leading context

If a future benchmark shows that tail context materially improves answer quality, that can be a separate iteration.

**Truncation Signaling**

The user follow-up prompt currently decides whether to add the truncation hint by inspecting the original tool-result text. That is insufficient once follow-up-only reduction mutates the rendered payload after the original tool result is produced.

This slice will therefore also update follow-up prompt hinting to consider both:

- the original tool-result text
- the rendered follow-up tool-result text

That keeps truncation guidance aligned with the actual payload shown to the model.

**Testing Strategy**

Add TDD coverage for:

- discovery-first follow-up reducing oversized `file.read` payload summaries
- turn-loop follow-up reducing oversized `file.read` payload summaries
- repeated-tool-guard replay reducing oversized `file.read` payload summaries
- `tool.search` follow-up payloads remaining unchanged
- truncation hints appearing when the rendered follow-up payload is newly marked truncated

**Risk Assessment**

This is a low-risk slice because it only changes model-facing follow-up assembly, not tool execution, persistence, provider routing, or raw-output delivery.

The main residual risk is overlap with other open follow-up payload PRs touching the same conversation files. That is a merge-management concern, not a runtime correctness concern.
