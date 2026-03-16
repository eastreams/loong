# Browser Automation

## User Story

As a LoongClaw operator, I want a minimal browser-like assistant capability so
that the agent can open public pages, extract structured content, and follow
safe links without needing a full desktop browser runtime.

## Acceptance Criteria

- [ ] LoongClaw exposes `browser.open`, `browser.extract`, and `browser.click`
      as bounded assistant tools.
- [ ] Browser tools share the same public-web safety model as `web.fetch`,
      including SSRF, private-host, domain-policy, redirect, and response-size
      guardrails.
- [ ] `browser.open` returns an opaque bounded session id plus page title,
      readable text, and discovered safe links.
- [ ] `browser.extract` supports first-MVP extraction modes for page text,
      title, safe links, and CSS selector text.
- [ ] `browser.click` can only follow links that were previously discovered in
      the active browser session.
- [ ] Browser session ids are scoped to the active conversation so one
      conversation cannot reuse or evict another conversation's browser state.
- [ ] Runtime-visible tool catalogs only advertise browser tools when the
      browser runtime is actually enabled under the current config.

## Out of Scope

- A full Chromium or Playwright runtime
- A managed browser automation companion with richer login, typing, screenshot,
  or form-completion actions
- Arbitrary JavaScript execution or login automation
- Form filling, uploads, or unrestricted browser macros
