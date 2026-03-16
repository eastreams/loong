# Doctor

## User Story

As a LoongClaw operator, I want a clear diagnostics and repair command so that I
can recover a broken setup without reverse-engineering runtime internals.

## Acceptance Criteria

- [ ] `loongclaw doctor` reports the health of the local assistant runtime in
      user-facing language.
- [ ] `loongclaw doctor --fix` only applies safe, local repair actions and
      explains what it changed.
- [ ] `loongclaw doctor --json` produces stable machine-readable output for
      automation and support tooling, including machine-readable `next_steps`
      when doctor can recommend a concrete repair or first-value command.
- [ ] Text-mode doctor output ends with concrete next actions such as
      credential env hints, `doctor --fix`, and first-turn ask/chat commands.
- [ ] When `onboard`, `ask`, `chat`, or channel setup hits a common health
      failure, the CLI points users toward `doctor`.
- [ ] Doctor checks cover the current MVP path: config presence, provider
      readiness, SQLite memory readiness, and shipped channel prerequisites.
- [ ] When `tools.browser_companion.enabled=true`, doctor surfaces companion
      install/runtime readiness as warnings with concrete repair steps instead
      of turning the optional managed lane into a hard core-runtime failure.

## Out of Scope

- Fully automatic repair for arbitrary operator customizations
- Remote fleet management
- Replacing onboarding as the preferred first-run path
