# Feishu Media Transfer Retry Design

**Goal:** Harden Feishu media upload/download stability by extending the Feishu-only retry layer to cover multipart upload paths and more complete `Retry-After` handling without changing non-Feishu runtime behavior.

**Scope:**
- Keep transport/routing boundaries unchanged.
- Keep retry behavior Feishu-only inside `crates/app/src/feishu/client.rs`.
- Keep upload/download resource logic inside `crates/app/src/feishu/resources/media.rs`.

**Chosen Approach:**
- Add TDD coverage for Feishu media transfer retry behavior:
  - multipart upload retries on retryable Feishu payload failures
  - binary download retries after rate limiting and still preserves metadata
  - `Retry-After` accepts HTTP-date format in addition to delta-seconds
- Refactor the Feishu client just enough to rebuild multipart requests per attempt instead of depending on `RequestBuilder::try_clone()` for bodies that may not be replayable.
- Continue to use the existing Feishu retry policy and retryable error classification so non-Feishu channels and generic tool runtime behavior remain untouched.

**Rejected Alternatives:**
- Add cross-channel retry middleware: rejected because it violates the current layering and would affect non-Feishu users.
- Push upload retry into `resources/media.rs` only: rejected because retry semantics belong in the Feishu client transport layer, not in each resource call site.
- Add proactive throttling in media resources first: rejected for this increment because it broadens behavioral change beyond the immediate stability gap.

**Risks and Guards:**
- Multipart replay can silently fail if the body cannot be cloned. Guard with a real retry regression test that forces a retryable first response.
- HTTP-date parsing can become flaky if tied to wall clock assumptions. Guard with a deterministic past-date test that should saturate to zero delay.
- Binary retry must not drop metadata. Guard with a retry test that verifies bytes and response headers after the second attempt.

**Validation Plan:**
- Focused `loongclaw-app` tests for new retry behaviors first.
- `cargo fmt --all`
- full `cargo test -p loongclaw-app`
- full `cargo test -p loongclaw-daemon`
- `git diff --check`
