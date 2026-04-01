# Weixin and QQBot Channel Bridge Support Design

**Scope**

This design deepens LoongClaw's support story for Chinese chat surfaces without
pretending that native runtime adapters already exist.

The immediate scope is:

- add first-class catalog surfaces for `weixin`, `qqbot`, and `onebot`
- model bridge-backed onboarding explicitly instead of collapsing everything
  into `planned`
- document `weixin` as a ClawBot-driven bridge path and `qqbot` as an official
  QQ Bot or plugin-driven bridge path
- define stable target and setup contracts that future adapters or plugins can
  reuse

This slice does not add native QR-code login, a built-in Weixin runtime, or a built-in
QQ gateway implementation.

**Problem Statement**

LoongClaw already ships real runtime-backed support for Telegram, Feishu/Lark,
Matrix, and WeCom, and it models many future channels in the broader catalog.

That is not enough for the current Chinese ecosystem use case:

- `weixin` is not modeled as a first-class surface even though the practical
  integration path is now ClawBot-style plugin bridging
- `qqbot` is not modeled as a first-class surface even though the ecosystem has
  real official QQ Bot and OpenClaw-style plugin bridges
- `onebot` is missing as a bridge-standard surface, which leaves a gap for the
  wider QQ and personal-account bridge ecosystem
- the channel catalog can currently say `runtime_backed`, `config_backed`, or
  `stub`, but it cannot truthfully say "supported through an external
  plugin/bridge contract"
- onboarding can currently say `manual_config` or `planned`, but it cannot
  express "LoongClaw knows this surface and its setup contract, but the active
  transport is expected to come from a plugin bridge"

The result is an avoidable mismatch:

- operators cannot see `weixin` or `qqbot` in the product's official channel
  inventory
- channel docs cannot point to a stable LoongClaw-owned contract for these
  surfaces
- plugin authors have no product-sanctioned target semantics to align with

**Reference Findings**

The external ecosystem is converging on the same pattern:

- OpenClaw publicly positions WeChat through the official ClawBot plugin path
  rather than through a built-in native runtime
- PicoClaw exposes Weixin, QQ, WeCom, and OneBot as distinct channel surfaces,
  which makes ecosystem breadth visible even when the transport strategy differs
  per surface
- `openclaw-qqbot` shows that the QQ Bot ecosystem needs explicit multi-account
  semantics, stable target formats, proactive send support, and media-aware
  operator guidance
- `weclaw` shows that ClawBot/iLink-style WeChat bridges benefit from clear
  bridge-first framing, stable per-user session routing, and a thin proactive
  send contract instead of pretending to own the whole upstream login stack

The useful product conclusion is not "clone those implementations". The useful
conclusion is:

1. LoongClaw should model `weixin`, `qqbot`, and `onebot` as real channel
   surfaces
2. those surfaces should be truthful about their current support mode
3. target and onboarding semantics should stabilize before any native adapter
   work starts

**Chosen Design**

## 1. Add a plugin-backed support tier to the channel model

LoongClaw should distinguish four implementation states:

- `runtime_backed`: built-in long-running runtime exists
- `config_backed`: built-in direct send support exists without built-in serve
- `plugin_backed`: LoongClaw models the surface and its setup contract, but the
  active transport is expected to come from an external plugin or bridge
- `stub`: metadata-only future surface with no sanctioned bridge path yet

This is the smallest truthful expansion of the current product language.

## 2. Add a plugin-bridge onboarding strategy

The onboarding contract should add a `plugin_bridge` strategy for surfaces where
LoongClaw can describe:

- the intended bridge family
- the stable configuration keys
- the target semantics
- the operator status entry point

while still not claiming native runtime ownership.

`weixin`, `qqbot`, and `onebot` should use this strategy.

## 3. Add first-class catalog surfaces for `weixin`, `qqbot`, and `onebot`

These surfaces should appear in `loongclaw channels --json` and the broader
catalog with stable ids, aliases, transports, and setup requirements.

Recommended identities:

- `weixin`
  aliases: `wechat`, `wx`, `wechat-clawbot`
  transport label: `wechat_clawbot_ilink_bridge`
- `qqbot`
  aliases: `qq`, `qq-bot`, `tencent-qq`
  transport label: `qq_official_bot_gateway_or_plugin_bridge`
- `onebot`
  aliases: `onebot-v11`, `napcat`, `llonebot`
  transport label: `onebot_v11_bridge`

## 4. Keep operations visible but explicitly non-native

Each new surface should declare `send` and `serve` operations in the catalog so
the product shape is complete, but both operations remain `stub` until a
LoongClaw-owned adapter exists.

That preserves two truths at once:

- LoongClaw knows what the surface wants to look like
- LoongClaw does not yet ship those commands natively

## 5. Publish stable target semantics now

The target contract should be written down now so later plugins and future
native adapters converge on one shape.

Recommended target families:

- `weixin`
  - direct contact: `weixin:<account>:contact:<id>`
  - group chat: `weixin:<account>:room:<id>`
- `qqbot`
  - private chat: `qqbot:<account>:c2c:<openid>`
  - group chat: `qqbot:<account>:group:<openid>`
  - guild channel: `qqbot:<account>:channel:<id>`
- `onebot`
  - private chat: `onebot:<account>:private:<user_id>`
  - group chat: `onebot:<account>:group:<group_id>`

LoongClaw does not need to parse these routes today to benefit from documenting
them. The key is to prevent ecosystem drift before future adapter work starts.

## 6. Use plugin metadata conventions instead of inventing a second plugin contract

LoongClaw's existing plugin manifest already has the important identity seam:

- `channel_id`
- `setup.surface`
- `setup.docs_urls`
- freeform `metadata`

This slice should not invent a second plugin manifest schema just for `weixin`
or `qqbot`.

Instead, the docs should standardize a bridge-oriented metadata convention for
channel plugins, such as:

- `channel_id = "weixin"` or `channel_id = "qqbot"`
- `setup.surface = "channel"`
- metadata keys that describe bridge kind and transport family
- docs URLs that point at operator setup docs

That keeps the plugin seam additive and avoids premature runtime coupling.

**Why This Is The Right Next Step**

This design improves product truthfulness without overcommitting the runtime:

- users can discover `weixin`, `qqbot`, and `onebot` through the official
  LoongClaw catalog
- docs can speak honestly about ClawBot and QQ Bot bridge paths
- plugin authors get a sanctioned surface id and setup story
- the product stops treating all non-native surfaces as indistinguishable
  `planned` stubs

Most importantly, it sets the stage for future work in the correct order:

1. stable surface ids and onboarding
2. stable target contracts
3. stable plugin metadata conventions
4. only then native runtime work where it is justified

That ordering is much safer than jumping straight to native adapters for
ecosystems whose practical integration path is still bridge-first.
