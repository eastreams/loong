# Channel Multi-Account Phase 8 Design

**Scope**

Phase 8 upgrades LoongClaw's Telegram and Feishu/Lark channel stack from
"account-aware identity" to actual multi-account configuration and selection.

This phase is still intentionally narrower than "implement Discord". It does
not add a Discord adapter, gateway, or monitor loop. It finishes the account
resolution substrate that OpenClaw already relies on across Telegram, Feishu,
and Discord.

**Problem Statement**

After Phase 7, LoongClaw can:

- derive a stable account identity from credentials
- scope runtime files by account identity
- scope Telegram offsets by account identity
- expose account-aware runtime info in `channels` and `doctor`

But LoongClaw still cannot do what OpenClaw's channel layers assume is normal:

- configure multiple accounts under one channel
- choose an explicit default account
- select one account at runtime from CLI
- show operator status per configured account instead of one platform-wide row

That means LoongClaw still behaves like a single-account system with nicer
labels. OpenClaw's Telegram, Feishu, and Discord implementations already sit on
top of a stricter pattern:

- account maps
- default-account resolution
- merged per-account config
- explicit account selection
- per-account status surfaces

Discord is downstream of that substrate. LoongClaw should finish that substrate
before any Discord work continues.

**Reference Findings From OpenClaw**

OpenClaw's Telegram and Feishu account modules share the same structure:

- `accounts` map stores per-account overrides
- `defaultAccount` resolves omitted account selection
- base config and account config are merged before runtime work starts
- helpers can list account ids, resolve default account id, and resolve one
  selected account

OpenClaw's Discord module follows the same pattern, not a special-case pattern.
That matters because the next missing layer in LoongClaw is architectural, not
transport-specific.

The useful conclusion is:

- Telegram, Feishu, and Discord all want one reusable account-resolution seam
- LoongClaw should implement that seam now for Telegram and Feishu
- Discord should wait until that seam is proven by config, startup, and
  operator surfaces

**Chosen Design**

Add actual account selection support for Telegram and Feishu/Lark while keeping
Phase 7's credential-derived identity model intact.

This phase separates two concepts:

1. configured account selector
   - chosen from `accounts.<id>` or fallback default selection
   - used by CLI selection and per-account status rows
2. resolved runtime identity
   - derived from merged config via explicit `account_id`, credential-derived
     identity, or `default`
   - used by session keys, runtime files, and Telegram offset persistence

This split preserves Phase 7 runtime compatibility while still enabling
OpenClaw-style multi-account config and selection.

**Config Changes**

Add to both `telegram` and `feishu`:

- `default_account`
- `accounts`

Add account override structs for both channels. Account override structs use
optional fields so they can override only the fields they need.

Examples of fields that remain overrideable per account:

- `enabled`
- `account_id`
- credentials / env pointers
- network endpoint settings
- allowlists
- Feishu webhook and receive-id settings

Top-level fields remain the base config. Account entries override them after
selection.

**Account Resolution Rules**

For each channel:

1. If `accounts` is non-empty:
   - valid configured account ids are the normalized account-map keys
   - omitted selection resolves to `default_account` when present and valid
   - otherwise fall back to the first sorted configured account id
2. If `accounts` is empty:
   - omitted selection resolves to current single-account behavior
   - the fallback configured account id remains compatible with Phase 7
3. After a configured account is selected:
   - merge base config with account override
   - resolve runtime account identity from the merged config

**CLI Changes**

Add explicit account selection flags:

- `telegram-serve --account <id>`
- `feishu-send --account <id>`
- `feishu-serve --account <id>`

If `--account` is omitted, runtime selection uses the configured default-account
rules above.

**Operator Surface Changes**

`channels` and `doctor` should move from one row per platform to one row per
configured account selection.

Each row should expose both:

- configured account selection id
- resolved runtime identity

This preserves Phase 7 observability while finally showing multiple configured
accounts separately.

**What This Phase Deliberately Does Not Solve**

This phase still does not add:

- multi-account concurrent serve fanout in one process
- Feishu multi-account shared webhook multiplexing
- generic monitor traits for websocket-style channels
- Discord adapter/runtime implementation

Those are later phases. Phase 8 is the last major config/runtime substrate step
before Discord becomes technically reasonable.

**Why This Is The Right Next Step**

Phase 8 closes the largest remaining gap between LoongClaw and OpenClaw's
channel architecture:

- config becomes truly multi-account
- startup becomes explicitly account-selectable
- registry becomes per-account instead of per-platform
- `doctor` and `channels` stop hiding account multiplicity

Once this exists, the next remaining Discord blocker is no longer basic account
resolution. It becomes the heavier runtime and monitor substrate Discord needs.
