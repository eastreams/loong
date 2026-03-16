# WebChat

## User Story

As a prospective LoongClaw user, I want a browser-facing chat surface so that I
can use the assistant without staying in a terminal.

## Acceptance Criteria

- [ ] Product docs state clearly whether WebChat is shipped, experimental, or
      planned. The MVP must not advertise it as generally available before it
      exists.
- [ ] Until WebChat ships, first-run docs direct users to a concrete
      `loongclaw ask --message "..."` example first and `loongclaw chat`
      second, instead of implying a browser UI already exists.
- [ ] When WebChat does ship, it must reuse the same conversation, provider,
      tool, and memory semantics as CLI ask/chat rather than creating a separate
      assistant runtime.
- [ ] WebChat is treated as the next user-facing surface after the base CLI
      path, not as a replacement for onboarding or doctor.

## Out of Scope

- Implementing WebChat in this change set
- Dashboard or multi-user admin controls
- Mobile or desktop companion apps
