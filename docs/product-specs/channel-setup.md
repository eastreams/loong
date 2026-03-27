# Channel Setup

## User Story

As a user who wants LoongClaw outside the terminal, I want channel setup to be
legible so that I know which surfaces are available today and what each one
needs.

## Acceptance Criteria

- [ ] Product docs clearly distinguish the shipped MVP surfaces:
      CLI as the default surface, plus runtime-backed Telegram, Feishu / Lark,
      Matrix, and WeCom, and config-backed outbound Discord, Slack, LINE,
      DingTalk, WhatsApp, Email, generic Webhook, Google Chat, Signal, Twitch,
      Microsoft Teams, Mattermost, Nextcloud Talk, Synology Chat, IRC,
      iMessage / BlueBubbles, and Nostr.
- [ ] Product docs clearly distinguish runtime-backed shipped surfaces,
      config-backed outbound shipped surfaces, and catalog-only planned
      surfaces such as Tlon, Zalo, Zalo Personal, and WebChat.
- [ ] Channel setup guidance describes required credentials, config toggles, and
      the command used to run each shipped channel today.
- [ ] Product docs describe `multi-channel-serve` as the current attached
      runtime owner for shipped runtime-backed surfaces and as the precursor to
      a broader gateway service layer rather than the long-term product noun.
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

- Shipping additional runtime-backed channels beyond CLI, Telegram, Feishu /
  Lark, Matrix, and WeCom
- Promoting the remaining catalog-only planned surfaces such as Tlon, Zalo,
  Zalo Personal, or WebChat to shipped support in this slice
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
| Discord | Config-backed outbound | Discord HTTP API | `discord.enabled`, `discord.bot_token` | `loongclaw discord-send` |
| Slack | Config-backed outbound | Slack Web API | `slack.enabled`, `slack.bot_token` | `loongclaw slack-send` |
| LINE | Config-backed outbound | LINE Messaging API | `line.enabled`, `line.channel_access_token` | `loongclaw line-send` |
| DingTalk | Config-backed outbound | DingTalk custom robot webhook | `dingtalk.enabled`, `dingtalk.webhook_url`; `secret` is optional when the webhook uses signed requests | `loongclaw dingtalk-send` |
| WhatsApp | Config-backed outbound | WhatsApp Cloud API | `whatsapp.enabled`, `whatsapp.access_token`, `whatsapp.phone_number_id` | `loongclaw whatsapp-send` |
| Email | Config-backed outbound | SMTP relay or SMTP URL | `email.enabled`, `email.smtp_host`, `email.smtp_username`, `email.smtp_password`, `email.from_address` | `loongclaw email-send` |
| Webhook | Config-backed outbound | generic HTTP webhook POST | `webhook.enabled`, `webhook.endpoint_url`; `auth_token` is optional and can pair with custom header and prefix overrides | `loongclaw webhook-send` |
| Google Chat | Config-backed outbound | Google Chat incoming webhook | `google_chat.enabled`, `google_chat.webhook_url` | `loongclaw google-chat-send` |
| Signal | Config-backed outbound | signal-cli REST bridge | `signal.enabled`, `signal.service_url`, `signal.account` | `loongclaw signal-send` |
| Twitch | Config-backed outbound | Twitch Chat API | `twitch.enabled`, `twitch.access_token`; optional `api_base_url`, `oauth_base_url`, and `channel_names` remain available for controlled environments and planned serve work | `loongclaw twitch-send` |
| Microsoft Teams | Config-backed outbound | Teams incoming webhook | `teams.enabled`, `teams.webhook_url` for sends; future bot runtime fields keep `teams.app_id`, `teams.app_password`, `teams.tenant_id`, `teams.allowed_conversation_ids` reserved for the planned serve path | `loongclaw teams-send` |
| Mattermost | Config-backed outbound | Mattermost REST API | `mattermost.enabled`, `mattermost.server_url`, `mattermost.bot_token` | `loongclaw mattermost-send` |
| Nextcloud Talk | Config-backed outbound | Nextcloud Talk bot API | `nextcloud_talk.enabled`, `nextcloud_talk.server_url`, `nextcloud_talk.shared_secret` | `loongclaw nextcloud-talk-send` |
| Synology Chat | Config-backed outbound | Synology Chat incoming webhook | `synology_chat.enabled`, `synology_chat.incoming_url` | `loongclaw synology-chat-send` |
| IRC | Config-backed outbound | IRC socket client | `irc.enabled`, `irc.server`, `irc.nickname`; `password` is optional, and `username`, `realname`, `channel_names` are optional operator hints | `loongclaw irc-send` |
| iMessage / BlueBubbles | Config-backed outbound | BlueBubbles bridge REST API | `imessage.enabled`, `imessage.bridge_url`, `imessage.bridge_token` | `loongclaw imessage-send` |
| Nostr | Config-backed outbound | relay publish over WebSocket | `nostr.enabled`, `nostr.relay_urls`, `nostr.private_key`; `allowed_pubkeys` stays reserved for the planned inbound path | `loongclaw nostr-send` |

## Expansion Model

LoongClaw keeps channel expansion in four explicit layers so planned surfaces
do not overclaim runtime support:

- the channel catalog is the superset and can model planned surfaces before a
  runtime adapter exists
- config-backed outbound surfaces are a shipped subset that own credentials,
  status, and direct sends without pretending they also own a long-running
  serve runtime
- runtime-backed service channels are a strict shipped subset of the catalog
- `multi-channel-serve` is the current attached runtime-owner precursor and
  only supervises enabled runtime-backed channels while using repeatable
  `--channel-account <channel=account>` selectors instead of channel-specific
  flags
- the longer-term direction is an explicit gateway service that will own
  runtime-backed channels, route mounts, auth, detached lifecycle, and operator
  APIs without changing the registry-first channel inventory model

This lets the product align channel naming and onboarding with broader channel
ecosystems such as OpenClaw without pretending a stub catalog entry or a
send-only surface is already a shipped runtime surface.

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

### Config-Backed Outbound Surfaces

Discord, Slack, LINE, DingTalk, WhatsApp, Email, generic Webhook, Google
Chat, Signal, Twitch, Microsoft Teams, Mattermost, Nextcloud Talk, Synology
Chat, IRC, iMessage / BlueBubbles, and Nostr are shipped as account-aware
outbound
surfaces:

- they publish send commands, config validation, inventory snapshots, and
  onboarding metadata through the shared channel SDK
- they do not join `multi-channel-serve` because they do not own a shipped
  reply-loop runtime
- their `serve` metadata remains planned or unsupported until the gateway layer
  and the underlying inbound transport contract are implemented

### Webhook

Generic Webhook is shipped as a minimal config-backed outbound POST surface:

- configure `webhook.endpoint_url` or account-specific
  `webhook.accounts.<account>.endpoint_url`
- optionally configure `auth_token`, `auth_header_name`, and
  `auth_token_prefix` when the remote endpoint expects bearer-like or custom
  header authentication
- use `webhook.payload_format = "json_text"` to send a JSON object with one
  text field, or `webhook.payload_format = "plain_text"` to send the raw body
- use `webhook.payload_text_field` to control the JSON field name for
  `json_text` payloads
- use `webhook-send` without `--target` to send to the configured endpoint, or
  override the endpoint with `--target` for one-off delivery
- `webhook.public_base_url` and `webhook.signing_secret` remain reserved for
  the planned inbound serve contract and are not required for send readiness

### Email

Email is shipped through an SMTP outbound surface:

- configure `email.smtp_host`, `email.smtp_username`, `email.smtp_password`,
  and `email.from_address`
- `email.smtp_host` may be either a bare relay host such as
  `smtp.example.com` or a full `smtp://` or `smtps://` URL when the operator
  needs an explicit port
- use `email-send` with an email address target
- the outbound surface sends plain-text mail and derives the subject from the
  first non-empty line of the message body
- `email-serve` remains planned until LoongClaw owns an IMAP-backed reply-loop
  runtime

### Microsoft Teams

Microsoft Teams is shipped through the incoming webhook send surface:

- configure `teams.webhook_url` for outbound delivery
- use `teams-send` without an explicit target to post into the configured
  webhook, or override the webhook with `--target` when the operator needs a
  one-off endpoint
- `teams.app_id`, `teams.app_password`, `teams.tenant_id`, and
  `teams.allowed_conversation_ids` remain reserved for the planned bot event
  runtime and are not required for send readiness today
- `teams-serve` remains planned until LoongClaw owns the bot-framework style
  inbound contract

### Twitch

Twitch is shipped through the official Twitch Chat API send surface:

- configure `twitch.access_token` with a Twitch user access token that carries
  `user:write:chat`
- use `twitch-send` with a channel login or broadcaster id target
- LoongClaw validates the token at send time to derive the sender user id and
  client id instead of duplicating those identifiers in config
- `twitch.api_base_url` and `twitch.oauth_base_url` stay overridable for tests
  and controlled environments
- `twitch.channel_names` remains reserved for the planned EventSub or
  chat-listener serve path

### Nextcloud Talk

Nextcloud Talk is shipped through the official bot API send surface:

- configure `nextcloud_talk.server_url` and `nextcloud_talk.shared_secret`
- use `nextcloud-talk-send` with a conversation token target
- `nextcloud-talk-serve` remains planned until LoongClaw owns the inbound bot
  callback contract

### Synology Chat

Synology Chat is shipped through the incoming webhook send surface:

- configure `synology_chat.incoming_url`
- use `synology-chat-send` with no explicit target to post into the webhook's
  bound room
- optionally pass a numeric user id target when the operator wants the webhook
  to direct-message a specific Synology Chat user
- `synology_chat.token` is reserved for a future outbound webhook serve
  contract and is not required for send readiness today
- `synology-chat-serve` remains planned until LoongClaw owns the outbound
  webhook callback contract

### IRC

IRC is shipped through a config-backed socket send surface:

- configure `irc.server` with either a bare host, an `irc://host[:port]`
  endpoint, or an `ircs://host[:port]` endpoint
- configure `irc.nickname` for the bot identity used during registration
- optionally configure `irc.username`, `irc.realname`, and `irc.password`
- when `irc.password` is set, use an `ircs://` endpoint so LoongClaw does not
  send `PASS` over plaintext transport
- optionally configure `irc.channel_names` when the operator wants status
  snapshots to advertise the preferred channel set for that account
- use `irc-send` with a single conversation target such as `#ops` for a
  channel or a nick for a direct message
- `irc-serve` remains planned until LoongClaw owns a long-lived relay-loop
  contract for IRC traffic

### iMessage / BlueBubbles

iMessage is shipped through a BlueBubbles bridge send surface:

- configure `imessage.bridge_url` and `imessage.bridge_token`
- use `imessage-send` with a BlueBubbles `chatGuid` target
- `imessage.allowed_chat_ids` remains reserved for a future inbound bridge-sync
  runtime and is not required for send readiness today
- `imessage-serve` remains planned until LoongClaw owns the inbound bridge
  synchronization contract

### Nostr

Nostr is shipped as a signed relay-publish surface:

- configure one or more relay URLs through `nostr.relay_urls`
- configure a signing key through `nostr.private_key`; both raw hex and `nsec`
  input are accepted, but LoongClaw normalizes internally to the standard hex
  representation
- use `nostr-send` to publish a regular text-note event and wait for relay `OK`
  acknowledgements from the configured relay set
- `nostr-send` may omit `--target` for a plain public note, or pass a public
  key target to attach a `p` tag to the outbound event
- `nostr.allowed_pubkeys` remains reserved for the planned inbound relay
  subscriber path and is not required for send readiness today

### Multi-Channel Serve And Gateway Direction

`multi-channel-serve` is the current attached runtime owner for the shipped
service-channel subset. It is also the first precursor to the planned explicit
gateway service rather than the long-term product noun:

- it keeps the concurrent CLI host in the foreground
- it supervises every enabled runtime-backed surface from the loaded config
- it accepts repeatable `--channel-account <channel=account>` selectors to pin
  specific accounts such as `telegram=bot_123456`, `lark=alerts`, `matrix=bridge-sync`,
  or `wecom=robot-prod`
- it never promotes config-backed outbound surfaces such as WhatsApp, Signal,
  Email, generic Webhook, Microsoft Teams, DingTalk, Google Chat,
  Mattermost, Nextcloud Talk, Synology Chat, IRC, or iMessage / BlueBubbles
  into runtime supervision until those adapters grow real serve ownership
- it never promotes catalog-only planned surfaces such as Tlon, Zalo,
  Zalo Personal, or WebChat into runtime supervision until those adapters are
  implemented
- the later gateway service should absorb this runtime ownership model, then
  add detached service lifecycle, route mounting, status/log surfaces, pairing,
  and richer gateway-native channel runtimes on top of the same registry-driven
  inventory contract
