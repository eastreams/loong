# Product Specs

User-facing product requirements and specifications for LoongClaw.

## Structure

Product specs describe **what** the product does from the user's perspective, not implementation internals or scheduling details.

## Specs

# Session And Delegate Tool Surface

## User Story
As an operator using LoongClaw's tool-calling runtime, I want to inspect active sessions and delegate focused subtasks into child sessions so that I can keep orchestration explicit, auditable, and bounded.

## Acceptance Criteria
- [x] Root sessions expose `sessions_list`, `sessions_history`, `session_status`, `session_events`, `session_cancel`, `session_recover`, `session_wait`, `delegate`, and `delegate_async` when enabled in config.
- [x] Delegated child sessions run with a restricted tool surface derived from config rather than inheriting the full root tool set.
- [x] Delegated child sessions can use `session_status` and `sessions_history` for self-inspection only, and never gain `sessions_list`.
- [x] Nested delegation is bounded by `tools.delegate.max_depth` and enforced from session lineage, not by ad-hoc one-off checks.
- [x] When nested delegation is allowed, child sessions only see `delegate` and `delegate_async` while they still have remaining depth budget.
- [x] Session visibility for `tools.sessions.visibility = "children"` includes the current session plus descendant delegate sessions.
- [x] Delegate child terminal outcomes are durably persisted and available through session inspection tools.
- [x] Legacy sessions that only exist in `turns` can still surface their own session summary without rewriting old rows.
- [x] `delegate_async` returns a child session id handle immediately, without waiting for worker launch completion, and child execution becomes observable through `session_status`, `session_events`, and `session_wait`.
- [x] `session_wait` can optionally continue an event cursor via `after_id` and return the full unseen incremental tail plus `next_after_id` together with the wait snapshot, including the terminal event when the session completes during the wait.
- [x] `session_status` and `session_wait` expose machine-readable terminal outcome record state plus normalized recovery metadata, preferring structured recovery events and falling back to synthesized `last_error` metadata when recovery event persistence also fails.
- [x] `session_status` and `session_wait` expose a normalized `delegate_lifecycle` summary for real delegate children, including queued vs running phase, inline vs async mode, and timeout-based staleness hints when the child is still non-terminal.
- [x] `sessions_list` supports machine-readable filtering for visible-session discovery and can surface `delegate_lifecycle` metadata when requested or when filtering overdue delegate children.
- [x] `session_status` and `session_wait` surface pending cancellation metadata for running async delegate children after an operator requests cancellation.
- [x] `session_cancel` can immediately cancel a visible queued async delegate child and can request cooperative cancellation for a visible running async delegate child without broadening child-session authority.
- [x] `session_recover` can mark a visible overdue queued or overdue running async delegate child as failed and persist both a terminal outcome and structured recovery event without broadening child-session authority.

## Current Limits
- `delegate_async` uses a subprocess one-shot worker (`loongclawd run-turn`) rather than a durable queue or resident worker pool.
- Child session inspection is self-only. A delegated child cannot browse descendants or list the session tree even when nested delegation is enabled.
- `session_cancel` cancels queued async children immediately, but running cancellation is cooperative at turn-loop checkpoints rather than hard process preemption.
- `session_recover` only handles overdue async delegate children in `ready` or `running`; it is an operator-driven recovery path, not hard kill, retry, or automatic restart recovery.
- `session_wait` is bounded polling over sqlite-backed session state, not a push stream.
- `sessions_list` is a bounded filtered snapshot, not a paginated or push-based session inventory stream.
- Async delegation has no hard kill, retry queue, or post-restart recovery semantics in this phase.
- Legacy fallback is best-effort for the current session only. Historical rows without `sessions` metadata cannot recover descendant lineage because `turns` do not encode parentage.
- Child tool allowlists only activate runtime-supported tools. Unknown or planned tool names are ignored.

## Out of Scope
- Durable delegate queues or leased worker pools
- Hard process kill, retries, or push subscriptions for child sessions
- Historical backfill or schema migration for old session lineage
- Exposing session-tree browsing tools such as `sessions_list` to delegated child sessions
