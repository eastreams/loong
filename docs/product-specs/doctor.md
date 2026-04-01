# Doctor

## User Story

As a LoongClaw operator, I want a clear diagnostics and repair command so that I
can recover a broken setup without reverse-engineering runtime internals.

## Acceptance Criteria

- [ ] `loong doctor` reports the health of the local assistant runtime in
      user-facing language.
- [ ] `loong doctor --fix` only applies safe, local repair actions and
      explains what it changed.
- [ ] `loong doctor --json` produces stable machine-readable output for
      automation and support tooling, including machine-readable `next_steps`
      when doctor can recommend a concrete repair or first-value command.
- [ ] `loong doctor security` provides a separate security exposure and
      config hygiene audit instead of overloading the general health report.
- [ ] `loong doctor security --json` emits a stable machine-readable
      contract with `command`, `config`, `ok`, `summary`, and `findings`.
- [ ] Security findings use the explicit posture vocabulary
      `covered | partial | exposed | unknown` so operators can distinguish
      strong coverage from soft guardrails and unresolved surfaces.
- [ ] Text-mode doctor output ends with concrete next actions such as
      credential env hints, `doctor --fix`, first-turn ask/chat commands, and
      optional browser preview enable or runtime setup commands when relevant.
- [ ] On a healthy setup, the first-turn recommendations read like the next user
      action, not just a status report, for example "Get a first answer" and
      "Continue in chat".
- [ ] When `onboard`, `ask`, `chat`, or channel setup hits a common health
      failure, the CLI points users toward `doctor`.
- [ ] Doctor checks cover the current MVP path: config presence, provider
      readiness, SQLite memory readiness, shipped channel prerequisites, and
      the optional browser preview companion readiness path.
- [ ] `doctor security` covers the current operator-facing security posture:
      audit retention durability, shell default policy, explicit tool file
      root, web fetch egress, external skills download posture, secret storage
      hygiene, and browser automation readiness posture.
- [ ] `doctor security` rejects `--fix` and `--skip-model-probe` instead of
      silently accepting unsupported parent-command flags.
- [ ] Durable audit readiness checks exercise the runtime `open + lock + unlock`
      path for JSONL retention instead of relying on metadata-only validation.
- [ ] When `tools.browser_companion.enabled=true`, doctor surfaces companion
      install/runtime readiness as warnings with concrete repair steps instead
      of turning the optional managed lane into a hard core-runtime failure.
- [ ] When provider model probing fails before any HTTP status is returned,
      doctor surfaces request/models host route diagnostics, including DNS
      results, fake-ip-style address detection, and a short TCP reachability
      probe with concrete repair guidance.

## Out of Scope

- Fully automatic repair for arbitrary operator customizations
- Remote fleet management
- Replacing onboarding as the preferred first-run path
