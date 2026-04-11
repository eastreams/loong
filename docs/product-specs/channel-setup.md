# Channel Setup

## User Story

As a user who wants LoongClaw outside the terminal, I want channel setup to be
legible so that I know which surfaces are available today and what each one
needs.

## Acceptance Criteria

- [ ] Product docs clearly distinguish the shipped MVP surfaces:
      CLI as the default surface, plus runtime-backed Telegram, Feishu / Lark,
      Matrix, and WeCom, and config-backed outbound Discord, Slack, LINE,
      DingTalk, WhatsApp, Email, generic Webhook, Google Chat, Signal,
      Microsoft Teams, Mattermost, Nextcloud Talk, Synology Chat,
      iMessage / BlueBubbles, plus plugin-backed bridge surfaces for Weixin,
      QQBot, and OneBot.
- [ ] Product docs clearly distinguish runtime-backed shipped surfaces,
      config-backed outbound shipped surfaces, plugin-backed bridge surfaces,
      and catalog-only planned surfaces such as IRC, Nostr, Twitch, Tlon,
      Zalo, Zalo Personal, and WebChat.
- [ ] Channel setup guidance describes required credentials, config toggles, and
      the command used to run each shipped channel.
- [ ] Channel setup guidance publishes stable target contracts for
      plugin-backed bridge surfaces such as `weixin`, `qqbot`, and `onebot`
      without claiming that LoongClaw already owns their native runtime.
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
- Promoting plugin-backed bridge surfaces such as Weixin, QQBot, or OneBot to
  native runtime-backed support in this slice
- Promoting the remaining catalog-only planned surfaces such as IRC, Nostr,
  Twitch, Tlon, Zalo, Zalo Personal, or WebChat to
  shipped support in this slice
- Broad cross-channel inbox or routing UX
- Full remote pairing flows for unshipped surfaces

## Channel Surface Matrix

| Surface | Status | Transport | Required config | Operator commands |
| --- | --- | --- | --- | --- |
| CLI | Shipped | local interactive runtime | none beyond base provider config | `loong ask`, `loong chat` |
| Telegram | Runtime-backed | Bot API polling | `telegram.enabled`, `telegram.bot_token`, `telegram.allowed_chat_ids` | `loong telegram-send`, `loong telegram-serve` |
| Feishu / Lark | Runtime-backed | webhook or websocket | `feishu.enabled`, `feishu.app_id`, `feishu.app_secret`, `feishu.allowed_chat_ids`; webhook mode also needs `verification_token` and `encrypt_key` | `loong feishu-send`, `loong feishu-serve` |
| Matrix | Runtime-backed | Client-Server sync | `matrix.enabled`, `matrix.access_token`, `matrix.base_url`, `matrix.allowed_room_ids` | `loong matrix-send`, `loong matrix-serve` |
| WeCom | Runtime-backed | official AIBot long connection | `wecom.enabled`, `wecom.bot_id`, `wecom.secret`, `wecom.allowed_conversation_ids` | `loong wecom-send`, `loong wecom-serve` |
| Discord | Config-backed outbound | Discord HTTP API | `discord.enabled`, `discord.bot_token` | `loong discord-send` |
| Slack | Config-backed outbound | Slack Web API | `slack.enabled`, `slack.bot_token` | `loong slack-send` |
| LINE | Config-backed outbound | LINE Messaging API | `line.enabled`, `line.channel_access_token` | `loong line-send` |
| DingTalk | Config-backed outbound | DingTalk custom robot webhook | `dingtalk.enabled`, `dingtalk.webhook_url`; `secret` is optional when the webhook uses signed requests | `loong dingtalk-send` |
| WhatsApp | Config-backed outbound | WhatsApp Cloud API | `whatsapp.enabled`, `whatsapp.access_token`, `whatsapp.phone_number_id` | `loong whatsapp-send` |
| Email | Config-backed outbound | SMTP relay or SMTP URL | `email.enabled`, `email.smtp_host`, `email.smtp_username`, `email.smtp_password`, `email.from_address` | `loong email-send` |
| Webhook | Config-backed outbound | generic HTTP webhook POST | `webhook.enabled`, `webhook.endpoint_url`; `auth_token` is optional and can pair with custom header and prefix overrides | `loong webhook-send` |
| Google Chat | Config-backed outbound | Google Chat incoming webhook | `google_chat.enabled`, `google_chat.webhook_url` | `loong google-chat-send` |
| Signal | Config-backed outbound | signal-cli REST bridge | `signal.enabled`, `signal.service_url`, `signal.account` | `loong signal-send` |
| Twitch | Config-backed outbound | Twitch Chat API | `twitch.enabled`, `twitch.access_token` or `twitch.access_token_env`; optional `default_account`, `accounts`, `api_base_url`, `oauth_base_url`, and `channel_names` remain available for account routing, controlled environments, and planned serve work | `loong twitch-send` |
| Tlon | Config-backed outbound | Urbit ship HTTP poke API | `tlon.enabled`, `tlon.ship`, `tlon.url`, `tlon.code` | `loong tlon-send` |
| Microsoft Teams | Config-backed outbound | Teams incoming webhook | `teams.enabled`, `teams.webhook_url` for sends; future bot runtime fields keep `teams.app_id`, `teams.app_password`, `teams.tenant_id`, `teams.allowed_conversation_ids` reserved for the planned serve path | `loong teams-send` |
| Mattermost | Config-backed outbound | Mattermost REST API | `mattermost.enabled`, `mattermost.server_url`, `mattermost.bot_token` | `loong mattermost-send` |
| Nextcloud Talk | Config-backed outbound | Nextcloud Talk bot API | `nextcloud_talk.enabled`, `nextcloud_talk.server_url`, `nextcloud_talk.shared_secret` | `loong nextcloud-talk-send` |
| Synology Chat | Config-backed outbound | Synology Chat incoming webhook | `synology_chat.enabled`, `synology_chat.incoming_url` | `loong synology-chat-send` |
| IRC | Config-backed outbound | IRC socket client | `irc.enabled`, `irc.server`, `irc.nickname`; `password` is optional, and `username`, `realname`, `channel_names` are optional operator hints | `loong irc-send` |
| iMessage / BlueBubbles | Config-backed outbound | BlueBubbles bridge REST API | `imessage.enabled`, `imessage.bridge_url`, `imessage.bridge_token` | `loong imessage-send` |
| Nostr | Config-backed outbound | relay publish over WebSocket | `nostr.enabled`, `nostr.relay_urls`, `nostr.private_key`; `allowed_pubkeys` stays reserved for the planned inbound path | `loong nostr-send` |

### Plugin-Backed Bridge Surfaces

| Surface | Status | Bridge family | Sanctioned setup keys | Stable targets | Native command state |
| --- | --- | --- | --- | --- | --- |
| Weixin | Plugin-backed bridge | ClawBot or iLink-compatible WeChat bridge | `weixin.enabled`, `weixin.bridge_url`, `weixin.bridge_access_token`; optional `weixin.allowed_contact_ids` | `weixin:<account>:contact:<id>`, `weixin:<account>:room:<id>` | `weixin-send`, `weixin-serve` are catalog stubs until a native adapter exists |
| QQBot | Plugin-backed bridge | official QQ Bot gateway or compatible plugin bridge | `qqbot.enabled`, `qqbot.app_id`, `qqbot.client_secret`; optional `qqbot.allowed_peer_ids` | `qqbot:<account>:c2c:<openid>`, `qqbot:<account>:group:<openid>`, `qqbot:<account>:channel:<id>` | `qqbot-send`, `qqbot-serve` are catalog stubs until a native adapter exists |
| OneBot | Plugin-backed bridge | OneBot v11 bridge such as NapCat or LLOneBot | `onebot.enabled`, `onebot.websocket_url`, `onebot.access_token`; optional `onebot.allowed_group_ids` | `onebot:<account>:private:<user_id>`, `onebot:<account>:group:<group_id>` | `onebot-send`, `onebot-serve` are catalog stubs until a native adapter exists |

### Plugin-Backed Bridge Surfaces

| Surface | Status | Bridge family | Sanctioned setup keys | Stable targets | Native command state |
| --- | --- | --- | --- | --- | --- |
| Weixin | Plugin-backed bridge | ClawBot or iLink-compatible WeChat bridge | `weixin.enabled`, `weixin.bridge_url`, `weixin.bridge_access_token`; optional `weixin.allowed_contact_ids` | `weixin:<account>:contact:<id>`, `weixin:<account>:room:<id>` | `weixin-send`, `weixin-serve` are catalog stubs until a native adapter exists |
| QQBot | Plugin-backed bridge | official QQ Bot gateway or compatible plugin bridge | `qqbot.enabled`, `qqbot.app_id`, `qqbot.client_secret`; optional `qqbot.allowed_peer_ids` | `qqbot:<account>:c2c:<openid>`, `qqbot:<account>:group:<openid>`, `qqbot:<account>:channel:<id>` | `qqbot-send`, `qqbot-serve` are catalog stubs until a native adapter exists |
| OneBot | Plugin-backed bridge | OneBot v11 bridge such as NapCat or LLOneBot | `onebot.enabled`, `onebot.websocket_url`, `onebot.access_token`; optional `onebot.allowed_group_ids` | `onebot:<account>:private:<user_id>`, `onebot:<account>:group:<group_id>` | `onebot-send`, `onebot-serve` are catalog stubs until a native adapter exists |

## Expansion Model

LoongClaw keeps channel expansion in four explicit implementation tiers inside
one channel catalog so surfaces do not overclaim runtime support:

- runtime-backed service channels own credentials, direct sends, status
  snapshots, and a long-running reply-loop runtime
- config-backed outbound surfaces own credentials, status, and direct sends
  without pretending they also own a long-running serve runtime
- plugin-backed bridge surfaces own sanctioned ids, onboarding hints,
  requirement keys, and target contracts while delegating active transport and
  login ownership to an external bridge
- stub surfaces remain metadata-only future entries with no sanctioned bridge
  contract yet

The channel catalog is the superset that can model all four tiers before every
adapter is shipped. `multi-channel-serve` only supervises enabled
runtime-backed channels and uses repeatable `--channel-account <channel=account>`
selectors instead of channel-specific flags.

This lets the product align channel naming and onboarding with broader channel
ecosystems without pretending a plugin bridge or stub catalog entry is already
a shipped native runtime surface.

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
- run `loong telegram-serve` for reply-loop automation
- use `loong telegram-send` for direct operator sends

### Feishu / Lark

Feishu supports two inbound transports and the security contract depends on the
selected mode:

- both webhook and websocket modes require `app_id`, `app_secret`, and
  `allowed_chat_ids`
- webhook mode additionally requires `verification_token` and `encrypt_key`
- websocket mode must not be blocked on webhook-only secrets
- websocket is the current default serve mode and does not open a local HTTP listener
- operators should be able to force the serve transport with `loong feishu serve --mode websocket|webhook`
- `loong feishu auth login` is the recommended human OAuth entry path; it may complete automatically through a loopback callback listener when `redirect_uri` is local
- `loong feishu auth login` should best-effort launch the authorize URL in a browser unless the operator disables that behavior
- `loong feishu auth start --json` and non-loopback redirect URIs remain explicit manual bootstrap flows
- `loong feishu-send` supports both `receive_id` and `message_reply`
- `loong feishu-serve` owns the inbound reply service

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
Chat, Signal, Twitch, Tlon, Microsoft Teams, Mattermost, Nextcloud Talk,
Synology
Chat, IRC, iMessage / BlueBubbles, and Nostr are shipped as account-aware
outbound
surfaces:

- they publish send commands, config validation, inventory snapshots, and
  onboarding metadata through the shared channel SDK
- they do not join `multi-channel-serve` because they do not own a shipped
  reply-loop runtime
- their `serve` metadata remains planned or unsupported until the gateway layer
  and the underlying inbound transport contract are implemented
- their HTTP targets must use `http` or `https`, must not embed credentials,
  block private or special-use hosts by default, and do not auto-follow
  redirects
- operators who intentionally send through a private bridge, loopback service,
  or self-hosted endpoint should set `[outbound_http] allow_private_hosts = true`
  at the top level of `loongclaw.toml`

### Plugin-Backed Bridge Surfaces

`weixin`, `qqbot`, and `onebot` are first-class channel catalog entries with
`implementation_status = "plugin_backed"` and
`onboarding.strategy = "plugin_bridge"`:

- they publish stable requirement metadata and target families through
  `loongclaw channels --json`
- they also expose config-derived account snapshots and bridge endpoint
  summaries through `loongclaw channels --json` when the bridge surface is
  configured
- their reserved native `*-send` and `*-serve` command ids remain non-runnable
  catalog stubs until LoongClaw ships the adapter itself
- they do not join `multi-channel-serve` because the active reply loop still
  belongs to the external bridge

### Weixin

`weixin` is a bridge-first surface that currently assumes a ClawBot or
iLink-compatible bridge:

- configure `weixin.bridge_url` and `weixin.bridge_access_token`
- optionally allowlist trusted contacts through `weixin.allowed_contact_ids`
- let the bridge own QR login, upstream session lifecycle, and personal WeChat
  transport details
- keep target routing stable with `weixin:<account>:contact:<id>` for direct
  contacts and `weixin:<account>:room:<id>` for group rooms

### QQBot

`qqbot` is a bridge-first surface for the official QQ Bot gateway or compatible
plugin transports:

- configure `qqbot.app_id` and `qqbot.client_secret`
- optionally allowlist trusted peers through `qqbot.allowed_peer_ids`
- treat account scope as part of the target contract because QQ openids are
  bot-account specific
- keep target routing stable with `qqbot:<account>:c2c:<openid>`,
  `qqbot:<account>:group:<openid>`, and `qqbot:<account>:channel:<id>`

### OneBot

`onebot` is the protocol bridge surface for QQ and personal-account ecosystems
that standardize on OneBot v11-compatible transports:

- configure `onebot.websocket_url` and `onebot.access_token`
- optionally allowlist trusted groups through `onebot.allowed_group_ids`
- keep the active WebSocket and event-loop ownership in the bridge runtime,
  not in LoongClaw
- keep target routing stable with `onebot:<account>:private:<user_id>` and
  `onebot:<account>:group:<group_id>`

### Plugin-Backed Bridge Surfaces

`weixin`, `qqbot`, and `onebot` are first-class channel catalog entries with
`implementation_status = "plugin_backed"` and
`onboarding.strategy = "plugin_bridge"`:

- they publish stable requirement metadata and target families through
  `loongclaw channels --json`
- they also expose config-derived account snapshots and bridge endpoint
  summaries through `loongclaw channels --json` when the bridge surface is
  configured
- `loongclaw doctor` validates the local bridge contract for these surfaces and
  treats external plugin runtime ownership as expected instead of as a native
  runtime failure
- their reserved native `*-send` and `*-serve` command ids remain non-runnable
  catalog stubs until LoongClaw ships the adapter itself
- they do not join `multi-channel-serve` because the active reply loop still
  belongs to the external bridge

When a plugin-backed bridge surface is configured and
`external_skills.install_root` is set, `loongclaw channels --json` also
surfaces a managed discovery block under `plugin_bridge_discovery`. That block
captures the last discovery snapshot LoongClaw can verify locally:

- `status`, `managed_install_root`, and `scan_issue` summarize discovery state
- `compatible_plugin_ids`, `compatible_plugins`, `incomplete_plugins`, and
  `incompatible_plugins` summarize bridge readiness at a glance
- `plugins[]` carries per-plugin facts such as `status`, `transport_family`,
  `target_contract`, `required_env_vars`, `required_config_keys`,
  `setup_docs_urls`, and `setup_remediation`

Compact example:

```json
{
  "catalog": {
    "id": "weixin",
    "implementation_status": "plugin_backed"
  },
  "plugin_bridge_discovery": {
    "status": "matches_found",
    "managed_install_root": "~/.loongclaw/managed-skills",
    "compatible_plugin_ids": ["weixin-bridge-a"],
    "compatible_plugins": 1,
    "incomplete_plugins": 0,
    "incompatible_plugins": 0,
    "plugins": [
      {
        "plugin_id": "weixin-bridge-a",
        "status": "compatible_ready",
        "transport_family": "wechat_clawbot_ilink_bridge",
        "target_contract": "weixin_reply_loop"
      }
    ]
  }
}
```

`loongclaw doctor` uses the same managed discovery contract. A compatible
external bridge counts as the expected runtime owner, while ambiguity,
incomplete bridge setup, or discovery scan failures remain operator-facing
warnings instead of being misreported as native adapter failures.

### Weixin

`weixin` is a bridge-first surface that currently assumes a ClawBot or
iLink-compatible bridge:

- configure `weixin.bridge_url` and `weixin.bridge_access_token`
- optionally allowlist trusted contacts through `weixin.allowed_contact_ids`
- let the bridge own QR login, upstream session lifecycle, and personal WeChat
  transport details
- keep target routing stable with `weixin:<account>:contact:<id>` for direct
  contacts and `weixin:<account>:room:<id>` for group rooms

### QQBot

`qqbot` is a bridge-first surface for the official QQ Bot gateway or compatible
plugin transports:

- configure `qqbot.app_id` and `qqbot.client_secret`
- optionally allowlist trusted peers through `qqbot.allowed_peer_ids`
- treat account scope as part of the target contract because openids are
  scoped to the selected QQ Bot account
- keep target routing stable with `qqbot:<account>:c2c:<openid>`,
  `qqbot:<account>:group:<openid>`, and `qqbot:<account>:channel:<id>`

### OneBot

`onebot` is the protocol bridge surface for QQ and personal-account ecosystems
that standardize on OneBot v11 compatible transports:

- configure `onebot.websocket_url` and `onebot.access_token`
- optionally allowlist trusted groups through `onebot.allowed_group_ids`
- keep the active WebSocket and event-loop ownership in the bridge runtime,
  not in LoongClaw
- keep `<account>` stable so personal-account bridge routes stay unambiguous
- keep target routing stable with `onebot:<account>:private:<user_id>` and
  `onebot:<account>:group:<group_id>`

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

### Signal

Signal is shipped through a `signal-cli` REST bridge send surface:

- configure `signal.account`
- use `signal.service_url` to point at the bridge; when unset, LoongClaw
  defaults to `http://127.0.0.1:8080`
- because outbound HTTP delivery defaults to public-only mode, the default
  local bridge requires `[outbound_http] allow_private_hosts = true`
- use `signal-send` with a Signal account target such as an E.164 number
- `signal-serve` remains planned until LoongClaw owns a real inbound listener
  contract

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

- enable the surface with `twitch.enabled = true`
- configure `twitch.access_token` or `twitch.access_token_env` with a Twitch
  user access token that carries `user:write:chat`
- use `twitch.account_id` when the operator wants an explicit runtime account
  identity label for the default config
- use `twitch.default_account` and `twitch.accounts.<account>` when the
  deployment needs multiple Twitch identities or environment-specific tokens
- use `twitch-send` with a channel login or broadcaster id target
- LoongClaw validates the token at send time to derive the sender user id and
  client id instead of duplicating those identifiers in config
- `twitch.api_base_url` and `twitch.oauth_base_url` stay overridable for tests
  and controlled environments
- `twitch.channel_names` remains reserved for the planned EventSub or
  chat-listener serve path

Example:

```toml
[twitch]
enabled = true
default_account = "ops"
channel_names = ["main-stream"]

[twitch.access_token]
env = "TWITCH_ACCESS_TOKEN"

[twitch.accounts.ops]
account_id = "twitch-ops"

[twitch.accounts.ops.access_token]
env = "TWITCH_OPS_ACCESS_TOKEN"

[twitch.accounts.backup]
enabled = false
account_id = "twitch-backup"
access_token_env = "TWITCH_BACKUP_ACCESS_TOKEN"
channel_names = ["backup-stream"]
```

Resolution notes:

- when `--account` is omitted, LoongClaw selects `twitch.default_account` if it
  is configured, otherwise it falls back to the single configured account or the
  sorted first account key
- `twitch.accounts.<account>.access_token` or
  `twitch.accounts.<account>.access_token_env` override the top-level token only
  for that account
- `twitch.accounts.<account>.account_id` overrides the top-level
  `twitch.account_id` for the resolved runtime identity

### Tlon

Tlon is shipped through the outbound Urbit ship poke surface:

- configure `tlon.ship`, `tlon.url`, and `tlon.code`
- `tlon.url` may omit the scheme; LoongClaw normalizes bare hosts to `https://`
- use `tlon-send` with DM targets such as `~sampel-palnet` or
  `dm:~sampel-palnet`
- use `tlon-send` with group targets such as `chat/~host-ship/channel` or
  `group:~host-ship/channel`
- LoongClaw authenticates to the configured ship, reuses the returned session
  cookie for one HTTP poke, and fails fast if login or poke acknowledgement
  does not succeed
- `tlon-serve` remains planned until LoongClaw owns a stable inbound Urbit
  subscription and reply-loop runtime

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

`gateway run/status/stop` is the current explicit owner contract for the
shipped runtime-backed service-channel subset. `multi-channel-serve` remains
the attached compatibility wrapper rather than the long-term product noun:

- `gateway run` can claim the persisted owner slot headless or attach a CLI
  host when `--session` is provided
- `gateway status` can inspect the persisted owner snapshot from another CLI
  process
- `gateway stop` can request cooperative shutdown from another CLI process
- `multi-channel-serve` uses the same gateway owner contract while preserving
  the attached CLI-first workflow for operators who want one foreground session

- `multi-channel-serve` keeps the concurrent CLI host in the foreground
- it supervises every enabled runtime-backed surface from the loaded config
- it accepts repeatable `--channel-account <channel=account>` selectors to pin
  specific accounts such as `telegram=bot_123456`, `lark=alerts`, `matrix=bridge-sync`,
  or `wecom=robot-prod`
- it never promotes config-backed outbound surfaces such as WhatsApp, Signal,
  Email, generic Webhook, Microsoft Teams, DingTalk, Google Chat,
  Mattermost, Nextcloud Talk, Synology Chat, or iMessage / BlueBubbles into
  runtime supervision until those adapters grow real serve ownership
- it never promotes plugin-backed bridge surfaces such as Weixin, QQBot, or
  OneBot into runtime supervision because their active event loop still belongs
  to the external bridge or gateway
- it never promotes catalog-only planned surfaces such as Tlon into runtime
  supervision until those adapters are implemented
- the gateway service should continue to absorb this runtime ownership model,
  then add detached service lifecycle, route mounting, status/log surfaces,
  pairing, and richer gateway-native channel runtimes on top of the same
  registry-driven inventory contract
