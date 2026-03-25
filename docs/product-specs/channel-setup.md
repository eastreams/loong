# Channel Setup

## User Story

As a user who wants LoongClaw outside the terminal, I want channel setup to be
legible so that I know which surfaces are available today and what each one
needs.

## Acceptance Criteria

- [ ] Product docs clearly distinguish the shipped MVP surfaces:
      CLI as the default surface, plus Telegram, Feishu / Lark, Matrix, and WeCom as optional channels.
- [ ] Product docs clearly distinguish runtime-backed shipped surfaces from
      catalog-only planned surfaces such as Discord, Slack, LINE,
      DingTalk, WhatsApp, Email, generic Webhook, Google Chat, Signal,
      Microsoft Teams, Mattermost, Nextcloud Talk, Synology Chat, IRC,
      iMessage / BlueBubbles, Nostr, Twitch, Tlon, Zalo, Zalo Personal,
      and WebChat.
- [ ] Channel setup guidance describes required credentials, config toggles, and
      the command used to run each shipped channel.
- [ ] WeCom setup guidance documents the official AIBot long-connection flow and
      never presents webhook callback mode as a supported LoongClaw integration path.
- [ ] Channel setup never implies a channel is ready until its required
      credentials and runtime prerequisites are satisfied.
- [ ] Channel-specific failures surface enough context for the operator to know
      which channel or account failed and how to recover.
- [ ] Channel setup guidance keeps the base CLI assistant path independent, so a
      user can still succeed with `ask` or `chat` before enabling service
      channels.

## Out of Scope

- Shipping additional channels beyond CLI, Telegram, Feishu / Lark, Matrix, and WeCom
- Promoting the remaining catalog-only planned surfaces such as Discord, Slack,
  DingTalk, WhatsApp, Signal, Synology Chat, Tlon, or iMessage / BlueBubbles
  to runtime-backed support in this slice
- Broad cross-channel inbox or routing UX
- Full remote pairing flows for unshipped surfaces

## Shipped Channel Matrix

| Surface | Status | Transport | Required config | Operator commands |
| --- | --- | --- | --- | --- |
| CLI | Shipped | local interactive runtime | none beyond base provider config | `loongclaw ask`, `loongclaw chat` |
| Telegram | Runtime-backed | Bot API polling | `telegram.enabled`, `telegram.bot_token`, `telegram.allowed_chat_ids` | `loongclaw telegram-send`, `loongclaw telegram-serve` |
| Feishu / Lark | Runtime-backed | webhook or websocket | `feishu.enabled`, `feishu.app_id`, `feishu.app_secret`, `feishu.allowed_chat_ids`; webhook mode also needs `verification_token` and `encrypt_key` | `loongclaw feishu-send`, `loongclaw feishu-serve` |
| Matrix | Runtime-backed | Client-Server sync | `matrix.enabled`, `matrix.access_token`, `matrix.base_url`, `matrix.allowed_room_ids` | `loongclaw matrix-send`, `loongclaw matrix-serve` |
| WeCom | Runtime-backed | official AIBot long connection | `wecom.enabled`, `wecom.bot_id`, `wecom.secret`, `wecom.allowed_conversation_ids` | `loongclaw wecom-send`, `loongclaw wecom-serve` |

## Expansion Model

LoongClaw keeps channel expansion in three explicit layers so planned surfaces
do not overclaim runtime support:

- the channel catalog is the superset and can model planned surfaces before a
  runtime adapter exists
- runtime-backed service channels are a strict shipped subset of the catalog
- `multi-channel-serve` only supervises enabled runtime-backed channels and uses
  repeatable `--channel-account <channel=account>` selectors instead of
  channel-specific flags

This lets the product align channel naming and onboarding with broader channel
ecosystems such as OpenClaw without pretending a stub catalog entry is already a
shipped runtime surface.

## Setup Rules

### CLI

The base CLI path stays independent from service channels. A user must be able
to succeed with `ask` or `chat` before enabling Telegram, Feishu, Matrix, or
WeCom.

### Telegram

Telegram setup remains the simplest shipped bot surface:

- enable the channel
- provide one bot token
- allowlist trusted chat ids
- run `loongclaw telegram-serve` for reply-loop automation
- use `loongclaw telegram-send` for direct operator sends

### Feishu / Lark

Feishu supports two inbound transports and the security contract depends on the
selected mode:

- both webhook and websocket modes require `app_id`, `app_secret`, and
  `allowed_chat_ids`
- webhook mode additionally requires `verification_token` and `encrypt_key`
- websocket mode must not be blocked on webhook-only secrets
- `loongclaw feishu-send` supports both `receive_id` and `message_reply`
- `loongclaw feishu-serve` owns the inbound reply service

### Matrix

Matrix uses a sync-loop transport with explicit homeserver configuration:

- configure `access_token` and `base_url`
- allowlist trusted room ids
- set `user_id` when self-message filtering is enabled
- use `matrix-send` for direct room delivery and `matrix-serve` for the sync
  reply loop

### WeCom

WeCom is shipped as a real runtime-backed surface through the official AIBot
long-connection transport:

- configure `bot_id` and `secret`
- allowlist trusted `conversation_id` values through
  `wecom.allowed_conversation_ids`
- use `wecom-serve` to own the long connection and auto-reply loop
- use `wecom-send` for proactive sends when no active `wecom-serve` session is
  holding the same bot account
- optional transport tuning belongs in `wecom.websocket_url`,
  `wecom.ping_interval_s`, and `wecom.reconnect_interval_s`

LoongClaw does not support a WeCom webhook callback mode on this surface. The
runtime contract is explicitly the official AIBot websocket subscription flow.

### Multi-Channel Serve

`multi-channel-serve` is the runtime owner for the shipped service-channel
subset:

- it keeps the concurrent CLI host in the foreground
- it supervises every enabled runtime-backed surface from the loaded config
- it accepts repeatable `--channel-account <channel=account>` selectors to pin
  specific accounts such as `telegram=bot_123456`, `lark=alerts`, `matrix=bridge-sync`,
  or `wecom=robot-prod`
- it never promotes catalog-only planned surfaces such as WhatsApp, Signal,
  DingTalk, Synology Chat, or Tlon into runtime supervision until those
  adapters are implemented
