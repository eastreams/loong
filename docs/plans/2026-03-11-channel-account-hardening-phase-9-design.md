# Channel Account Hardening Phase 9 Design

**Scope**

Phase 9 hardens the Telegram and Feishu/Lark multi-account substrate that
Phase 8 introduced.

This phase still does not add Discord transport work. It closes the remaining
config-integrity gap that would otherwise leak ambiguous account state into any
future Discord adapter, supervisor, or monitor layer.

**Problem Statement**

After Phase 8, LoongClaw can configure and select multiple Telegram and
Feishu/Lark accounts. That closes the largest architectural gap with
OpenClaw's channel model, but two integrity problems remain:

- normalized account-id collisions are silently deduplicated
- `default_account` silently falls back when it points at a missing account

Those behaviors are convenient in the short term but dangerous in the system
that LoongClaw is becoming:

- operator intent can drift from runtime selection without any hard signal
- `channels` and `doctor` can display a plausible status for the wrong logical
  account
- later channel plugins such as Discord would inherit the same ambiguity

**Reference Findings**

OpenClaw treats account selection as real channel architecture, not cosmetic
config sugar:

- shared account helpers normalize account ids before default resolution
- `defaultAccount` is expected to route work intentionally across Telegram,
  Feishu, and Discord
- upstream changelog entries repeatedly tighten default-account behavior and
  preserve account-resolution errors instead of hiding them

The useful conclusion is not "clone every fallback exactly". The useful
conclusion is that account selection is upstream-critical and must stay
observable and deterministic.

**Chosen Design**

Phase 9 makes config ambiguity fail early and makes the chosen default account
visible everywhere operators inspect channel state.

The design has three rules:

1. normalized configured account ids must be unique
2. when `accounts` is non-empty, `default_account` must resolve to one of those
   configured ids
3. status surfaces must explicitly mark which configured account is the default

This keeps valid configs fully compatible while blocking ambiguous configs
before runtime startup.

**Validation Rules**

For both `telegram` and `feishu`:

- normalize configured account ids with the same normalization logic used for
  runtime selection
- if two raw keys normalize to the same id, emit a structured validation
  diagnostic and fail config validation
- if `default_account` is set while `accounts` is non-empty, normalize it and
  require it to match one configured account id
- if `accounts` is empty, do not let `default_account` invent a synthetic
  configured-account id; single-account fallback should stay derived from the
  resolved runtime identity or `default`

**Operator Surface Changes**

`ChannelStatusSnapshot` should grow a boolean marker for default-account
selection. That marker should appear in:

- JSON output from `loongclaw channels --json`
- text output from `loongclaw channels`
- snapshot notes used by `doctor` and other diagnostics

The goal is not cosmetic labeling. The goal is to make it obvious which
configured account would be selected when `--account` is omitted.

**Why This Is The Right Next Step**

Discord is still downstream of this substrate. Without Phase 9, Discord would
inherit the same hidden ambiguity LoongClaw currently tolerates in Telegram and
Feishu/Lark:

- duplicate logical accounts collapsing to one normalized id
- invalid defaults silently changing target selection
- operator surfaces missing the actual default route

Phase 9 keeps the current scope narrow and causal:

- fail ambiguous config before runtime
- make default routing explicit in operator views
- preserve the multi-account architecture already implemented

That is the last low-level account-integrity step before LoongClaw should spend
more effort on shared runtime/supervisor substrate or any Discord-specific
adapter work.
