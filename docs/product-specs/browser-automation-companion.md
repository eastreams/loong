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
- [ ] The companion uses an isolated LoongClaw-managed browser profile by
      default instead of assuming access to the user's personal browser profile.
- [ ] Any bundled or preinstalled helper skill for browser automation is
      documented as guidance on top of the companion runtime, not as the source
      of truth for whether the capability is installed and supported.

## Out of Scope

- Replacing the shipped lightweight browser tools
- Making heavy browser automation part of the default install path
- WebChat, dashboard controls, or broader trigger automation
- Arbitrary desktop automation outside the browser surface
