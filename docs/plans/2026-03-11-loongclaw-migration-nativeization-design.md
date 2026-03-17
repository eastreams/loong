# LoongClaw Migration Nativeization Design

Date: 2026-03-11
Status: Approved and implemented for v0.1 importer

## Summary

LoongClaw needs a migration path for users coming from older claw-family
projects without forcing them to manually rebuild identity, tuning, and
long-term preferences from scratch.

The user requirement is specific:

- migrating users should keep their custom identity and prompt tuning
- stock upstream claw identity should not survive as-is
- after migration, the runtime identity should be LoongClaw, not OpenClaw,
  NanoBot, PicoClaw, ZeroClaw, or NanoClaw
- stock templates should be replaced by LoongClaw-native prompt assets rather
  than lightly renamed copies
- durable imported traits should survive in a structured place that fits
  LoongClaw's memory architecture

The v0.1 decision is to nativeize imported content into existing LoongClaw
surfaces instead of inventing a separate migration-only prompt stack.

## Upstream Research Findings

### NanoBot

- System prompt is composed from identity + bootstrap files + memory + skills.
- Stock identity and memory templates contain explicit `nanobot` branding.
- This makes NanoBot a strong fit for content-level migration rather than
  config-field translation.

### OpenClaw

- User shaping happens through workspace files such as `AGENTS.md`, `SOUL.md`,
  `IDENTITY.md`, `USER.md`, and `BOOTSTRAP.md`.
- Default templates are generic placeholders or onboarding scaffolds.
- OpenClaw config also contains some inline `identity` fields, but the
  workspace remains the most portable migration input.

### PicoClaw

- PicoClaw already ships a source/target migration pipeline for OpenClaw.
- Its workspace defaults are effectively a lightweight NanoBot-style prompt
  template with PicoClaw branding.
- This validates source detection + workspace scanning as the right product
  shape for LoongClaw too.

### ZeroClaw

- ZeroClaw builds system prompts from workspace bootstrap files or AIEOS
  identity JSON.
- Generated workspace files are opinionated and strongly branded, but still
  structurally portable.
- AIEOS payloads are useful imported identity sources even when the rest of the
  workspace should be replaced by LoongClaw-native prompt assets.

### NanoClaw

- NanoClaw uses `CLAUDE.md`-style prompt files rather than the same
  workspace/memory model as the other claws.
- It is still migratable as prompt content, but skills/runtime-specific group
  orchestration should not be treated as fully portable yet.

## Product Decision

Imported content is split into three buckets:

1. **Stock upstream templates**
   - discard the imported stock prompt content
   - switch the target config to LoongClaw native prompt pack
   - preserve LoongClaw safety/personality/memory defaults

2. **Prompt-level customizations**
   - preserve user-authored behavior/tone instructions as
     `cli.system_prompt_addendum`
   - normalize legacy claw branding references to `LoongClaw`

3. **Durable identity/preferences**
   - preserve imported identity/profile/memory notes as
     `memory.profile_note`
   - activate `memory.profile = "profile_plus_window"`
   - normalize legacy claw branding references to `LoongClaw`

This keeps the final runtime prompt LoongClaw-native while still inheriting the
user's custom setup.

## Why Existing LoongClaw Surfaces Are Enough

LoongClaw already has the right durable surfaces:

- `cli.prompt_pack_id`
- `cli.personality`
- `cli.system_prompt_addendum`
- `memory.profile`
- `memory.profile_note`

Using these means migration output is transparent, inspectable, and compatible
with onboarding/runtime behavior that already exists.

## Nativeization Rules

### Rule 1: Stock content becomes LoongClaw native

If imported content matches a known stock upstream template, do not keep it as
inline prompt text. Replace the effective identity with:

- LoongClaw prompt pack
- current/default LoongClaw personality
- LoongClaw memory profile defaults

### Rule 2: Custom content is preserved as overlays

If the content is not a stock template, preserve it in the smallest correct
surface:

- `AGENTS.md`, `SOUL.md`, `TOOLS.md`, `BOOTSTRAP.md`, `CLAUDE.md`
  -> prompt addendum
- `IDENTITY.md`, `USER.md`, `MEMORY.md`, `memory/MEMORY.md`, AIEOS identity
  -> memory profile note

### Rule 3: Brand references are normalized

When preserved content still references an upstream claw brand, rewrite only
the claw name to `LoongClaw`.

Examples:

- `nanobot` -> `LoongClaw`
- `OpenClaw` -> `LoongClaw`
- `PicoClaw` -> `LoongClaw`
- `ZeroClaw` -> `LoongClaw`
- `NanoClaw` -> `LoongClaw`

This preserves the user's meaning while removing conflicting self-identity.

### Rule 4: Do not silently migrate security-sensitive config

v0.1 deliberately does not import:

- API keys
- OAuth profiles
- external connector secrets
- hook/webhook credentials

Credentials should stay operator-managed and explicit.

## v0.1 Scope

### Implemented

- source detection for claw-family workspaces/config folders
- stock-template nativeization for NanoBot/OpenClaw/PicoClaw/ZeroClaw/NanoClaw
- prompt customization import into `system_prompt_addendum`
- durable identity import into `memory.profile_note`
- AIEOS identity JSON import into `profile_note`
- daemon CLI command for writing migrated LoongClaw config

### Deferred

- automatic skill migration
- heartbeat/scheduler task migration
- upstream credential migration
- deep parsing of every legacy config schema
- onboarding-integrated one-shot migration wizard

## Future Direction

The next product step is not a broader rename table. It is onboarding-level
integration:

- `loongclawd onboard --migrate <path>`
- personality selection applied after nativeization
- future pluggable memory backends still reading the same imported
  `profile_note`
- optional richer import reports that show exactly which files were nativeized
  vs preserved

That keeps the architecture aligned with LoongClaw-native prompt packs and
pluggable memory rather than building a permanent compatibility layer around
legacy prompt formats.
