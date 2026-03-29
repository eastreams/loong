# Browser Automation Companion

## User Story

As a LoongClaw user, I want an optional managed browser automation companion so
that the assistant can complete supported page tasks without turning the default
runtime into a heavyweight browser platform.

## Acceptance Criteria

- [ ] Product docs clearly distinguish the shipped safe browser lane
      (`browser.open`, `browser.extract`, `browser.click`) from the planned
      browser automation companion lane.
- [ ] The browser automation companion is treated as an optional enhanced
      capability with its own install, onboarding, and doctor readiness flow,
      not as a mandatory dependency for all LoongClaw users.
- [ ] When the companion is unavailable, unhealthy, disabled, or policy-blocked,
      its richer browser tools are not advertised in capability snapshots,
      provider tool schemas, or product-facing first-run guidance.
- [ ] When the companion does ship, it reuses LoongClaw's existing capability,
      approval, policy, and audit model rather than exposing a raw shell-only
      browser CLI.
- [ ] When the companion grows beyond the preview path, it attaches through the
      daemon-owned gateway or equivalent service-owned node model rather than
      creating a second long-lived runtime host.
- [ ] The companion uses an isolated LoongClaw-managed browser profile by
      default instead of assuming access to the user's personal browser profile.
- [ ] Any bundled or preinstalled helper skill for browser automation is
      documented as guidance on top of the companion runtime, not as the source
      of truth for whether the capability is installed and supported.

## Current Preview Scope

The currently shipped preview scope is narrower than the final managed browser
automation companion:

- a first-party bundled `browser-companion-preview` managed skill
- `loong skills enable-browser-preview` as the operator-facing fast path
- `loong skills enable-browser-preview` now returns concrete install,
  verify, and first-task recipe guidance instead of only install metadata
- when `cli.enabled=false`, the preview enable flow withholds ask-based recipes
  and tells the operator to re-enable the CLI before running them
- `onboard` and `doctor` next actions that surface the preview truthfully,
  including enable, install, verify, or first-recipe handoffs depending on
  readiness
- continued default shipping of only `browser.open`, `browser.extract`, and
  `browser.click` as built-in browser tools
- a partial governed adapter skeleton behind companion runtime readiness:
  - runtime-visible `browser.companion.session.start`,
    `browser.companion.navigate`, `browser.companion.snapshot`,
    `browser.companion.wait`, and `browser.companion.session.stop`
  - governed write actions `browser.companion.click` and
    `browser.companion.type` routed through the App-tool approval path
  - a structured command protocol, LoongClaw-issued companion session IDs, and
    session-scoped companion state instead of raw shell text passthrough
  - bounded companion command execution so hung adapter processes fail closed
    instead of wedging the turn
- the bundled `browser-companion-preview` managed skill acts as guidance on top
  of the runtime surface, not as the source of truth for runtime availability
- the longer-term target is to promote the companion into a gateway-managed node
  once explicit gateway service ownership lands

The install/release lifecycle, `onboard` and `doctor` readiness integration,
isolated browser profile management, stronger approval evidence UX, and broader
runtime packaging still remain planned work.

## Out of Scope

- Replacing the shipped lightweight browser tools
- Making heavy browser automation part of the default install path
- The broader Web UI surface or trigger automation beyond the browser companion scope
- Arbitrary desktop automation outside the browser surface
