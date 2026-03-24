# Channel Setup

## User Story

As a user who wants LoongClaw outside the terminal, I want channel setup to be
legible so that I know which surfaces are available today and what each one
needs.

## Acceptance Criteria

- [ ] Product docs clearly distinguish the shipped MVP surfaces:
      CLI as the default surface, plus Telegram, Feishu / Lark, and Matrix as optional channels.
- [ ] Product docs clearly distinguish runtime-backed shipped surfaces from
      catalog-only planned surfaces such as Discord, Slack, LINE, WeCom,
      DingTalk, WhatsApp, Email, and generic Webhook.
- [ ] Channel setup guidance describes required credentials, config toggles, and
      the command used to run each shipped channel.
- [ ] Channel setup never implies a channel is ready until its required
      credentials and runtime prerequisites are satisfied.
- [ ] Channel-specific failures surface enough context for the operator to know
      which channel or account failed and how to recover.
- [ ] Channel setup guidance keeps the base CLI assistant path independent, so a
      user can still succeed with `ask` or `chat` before enabling service
      channels.

## Out of Scope

- Shipping additional channels beyond CLI, Telegram, Feishu / Lark, and Matrix
- Promoting catalog-only planned surfaces to runtime-backed support in this slice
- Broad cross-channel inbox or routing UX
- Full remote pairing flows for unshipped surfaces
