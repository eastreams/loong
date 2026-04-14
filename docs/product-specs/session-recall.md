# Session Recall

## User Story

As a Loong operator, I want the assistant to search canonical session
history directly so that follow-up work can recall prior turns and session
events without forcing me to manually inspect transcripts.

## Acceptance Criteria

- [x] Loong exposes `session_search` as a runtime-visible session tool when
      session tools are enabled.
- [x] `session_search` searches only the visible session scope for the current
      session according to the existing session visibility policy.
- [x] `session_search` can search transcript turns and structured session
      events from canonical session history.
- [x] `session_search` supports narrowing to one visible `session_id`.
- [x] `session_search` excludes archived sessions by default and requires an
      explicit `include_archived=true` override to search archived visible
      sessions.
- [x] `session_search` returns ranked structured hits with enough metadata to
      identify the matched session, source kind, and snippet.
- [x] `session_search` requires `MemoryRead` rather than filesystem access
      because it searches Loong-owned canonical session history.
- [x] Product docs and tool-surface descriptions use the canonical tool name
      `session_search`.

## Out of Scope

- Semantic vector retrieval
- Cross-user or cross-operator global recall
- Automatic long-term memory summarization from search results
- Exporting canonical session history into a separate search daemon
