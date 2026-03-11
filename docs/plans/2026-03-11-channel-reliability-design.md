# Channel Reliability Phase 1 Design

**Scope**

This design covers the first implementation phase for LoongClaw channel hardening after the
Telegram / Feishu / Lark / Discord comparison study.

The goal is not to reproduce OpenClaw's full channel platform in one pass. The goal is to
remove the highest-risk delivery bugs in the current MVP and introduce one small channel-runtime
contract improvement that future Feishu/Lark and Discord work can build on without rework.

**Problems Being Solved**

1. Telegram currently persists the next polling offset before provider execution and outbound send
   have completed. A failure after polling can silently drop updates.
2. Feishu webhook dedupe currently marks an event as seen before provider execution and outbound
   reply have completed. A failure after dedupe can cause later retries to be ignored.
3. The current channel contract has no delivery acknowledgement hook, so reliability semantics are
   trapped inside each adapter and cannot evolve cleanly.

**Chosen Design**

Introduce a minimal delivery-ack layer instead of a large runtime rewrite:

- Extend `ChannelInboundMessage` with optional delivery metadata.
- Extend `ChannelAdapter` with default no-op hooks for:
  - per-message acknowledgement
  - end-of-batch completion
- Teach the Telegram adapter to:
  - keep polled offsets pending until processing succeeds
  - acknowledge successfully handled messages incrementally
  - advance trailing ignored updates only when the batch completes
- Teach the Feishu webhook dedupe cache to distinguish `processing` from `completed`
  and to release failed events so platform retries can be processed again.

**Why This Approach**

- It fixes the worst correctness bugs now.
- It avoids a risky, wide refactor of conversation/runtime boundaries in one patch.
- It creates a small but real seam for later channel-runtime enrichment.
- It preserves existing user-facing behavior except where retry/drop semantics improve.

**Deferred Work**

This phase intentionally does not implement:

- Feishu/Lark unified domain/config model
- Telegram webhook / callback query / topic routing
- Discord adapter
- full `ChannelEvent` / `OutboundMessage` runtime redesign

Those remain Phase 2+ work after the reliability layer is stable.
