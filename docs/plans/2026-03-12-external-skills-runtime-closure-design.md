# External Skills Runtime Closure Design

Date: 2026-03-12
Status: Approved for implementation

## Summary

LoongClaw already has the first half of an external-skills story:

- `external_skills.policy` guards runtime enablement, approval, and
  domain allow/block policy
- `external_skills.fetch` downloads artifacts under a managed directory
- migration can detect legacy skill catalogs and preserve them as durable
  profile metadata

What LoongClaw does not have yet is the second half:

- install a downloaded or local skill package into a managed runtime root
- list and inspect installed skills
- expose installed skills to the model in a deterministic way
- let the model explicitly load a skill's instructions into the conversation

The recommended design is to treat external skills as managed instruction
packages, not as dynamically generated native function tools.

That means:

- keep the built-in tool registry mostly static
- add explicit lifecycle tools for install, list, inspect, invoke, and remove
- surface installed skills as runtime context the model can discover
- make skill invocation return deterministic instruction payloads that the
  existing turn loop can feed back into later provider rounds

This closes the runtime loop without forcing a full dynamic provider-tool
registry or a new plugin execution bridge.

## Product Goals

- Close the runtime gap between external skill download and usable skill
  invocation.
- Keep the safety model explicit and deterministic.
- Preserve the current security posture:
  downloads remain opt-in, approval-gated, and domain-restricted.
- Keep skill installation auditable and reversible.
- Make installed skills discoverable by the model and by operators.
- Keep the implementation compatible with the current static built-in tool
  architecture.

## Non-Goals For This Slice

- No per-skill dynamic OpenAI function registration.
- No remote auto-install directly from a URL in a single step.
- No automatic execution of arbitrary scripts embedded in skill packages.
- No full ecosystem packaging/signing rollout yet.
- No hidden auto-mount of legacy migrated skills.

## Current State

Today the external-skills path stops in two places:

1. Download path
   - `external_skills.fetch` validates policy and writes raw bytes to
     `external-skills-downloads/`
2. Migration path
   - legacy `SKILLS.md`, `skills-lock.json`, `.codex/skills`, `.claude/skills`,
     and `skills/` artifacts are detected and summarized into
     `memory.profile_note`

This produces auditability but not a usable runtime.

The current conversation/provider/tool stack also assumes a static tool set:

- `tool_registry()` is static
- `provider_tool_definitions()` is static
- `is_known_tool_name()` rejects unknown names
- `execute_tool_core_with_config()` dispatches through a static match

That makes a "one installed skill = one dynamic function tool" design much
more invasive than it looks.

## Design Principle: Skills Are Managed Instruction Packages

In LoongClaw's current product shape, an external skill is closer to a
portable instruction bundle than to a native connector or executable plugin.

That aligns with existing migration research:

- upstream claws often compose prompts from identity, memory, bootstrap files,
  and skills
- migration already preserves skills as prompt/runtime context rather than as
  native adapters

So the runtime closure should preserve that semantic model:

- installation manages package lifecycle
- invocation loads instructions into the conversation
- execution still happens through the normal model + built-in tool loop

## Approaches Considered

### Approach 1: Dynamic Per-Skill Function Tools

Install each skill and turn it into a unique provider function definition.

Pros:

- feels like native tool calling
- the model can call specific skill names directly

Cons:

- requires dynamic provider schema generation
- requires dynamic known-tool gating and dynamic dispatch
- increases failure surface across provider, conversation, and core tools
- makes package metadata shape part of the model tool contract immediately

### Approach 2: Managed Lifecycle Tools Plus Explicit Skill Invocation

Keep the built-in tool set static and add:

- `external_skills.install`
- `external_skills.list`
- `external_skills.inspect`
- `external_skills.invoke`
- `external_skills.remove`

Installed skills are surfaced in capability context, and `invoke` returns the
resolved skill instructions and metadata for later model turns.

Pros:

- matches current architecture
- closes the loop without dynamic tool registration
- keeps auditability and testing straightforward
- preserves the semantic meaning of skills as instruction packages

Cons:

- the model has to call a generic tool rather than a per-skill function

### Approach 3: Prompt-Only Auto-Mount

Install skills and silently append all active skill instructions to the system
prompt or profile memory.

Pros:

- easiest runtime integration

Cons:

- hidden state and prompt bloat
- poor operator visibility
- no explicit invocation boundary
- hard to test and reason about

## Decision

Adopt Approach 2.

LoongClaw should add a managed external-skills lifecycle with explicit
installation and invocation tools, while keeping the provider-facing function
tool surface static.

## User-Facing Runtime Model

### 1. Download

The operator or model uses `external_skills.fetch` to download a package under
the managed downloads directory.

### 2. Install

The operator or model uses `external_skills.install` with either:

- a local directory containing `SKILL.md`
- a local `.tgz` / `.tar.gz` package

Installation:

- validates source path safety
- extracts archives into a temporary staging area
- locates a single skill root containing `SKILL.md`
- derives or validates a stable `skill_id`
- copies the normalized skill into a managed installs directory
- updates an installed-skill index

### 3. Discover

The operator or model uses `external_skills.list` to discover installed skills.

Each entry includes:

- `skill_id`
- source kind/path
- install path
- content summary
- whether the skill is currently active
- package digest if available

### 4. Inspect

The operator or model uses `external_skills.inspect` to read structured skill
metadata before invoking it.

### 5. Invoke

The operator or model uses `external_skills.invoke` with a `skill_id`.

The tool returns:

- resolved `skill_id`
- install path
- source metadata
- the loaded `SKILL.md` instructions
- a short invocation summary suitable for tool-result feedback

The existing conversation turn loop can then feed that tool result back into
later provider rounds, allowing the model to follow the skill instructions in a
controlled and explicit way.

### 6. Remove

The operator or model uses `external_skills.remove` to uninstall a managed
skill and update the index.

## Package Contract

This slice intentionally uses a minimal package contract.

### Supported Inputs

- local directory
- local `.tgz`
- local `.tar.gz`

### Required Content

- exactly one installable skill root
- root must contain `SKILL.md`

### Optional Content

- `assets/`
- `references/`
- `scripts/`
- supporting markdown/docs under the skill root

### Deferred Contract

This slice does not require:

- signed manifests
- semantic version fields
- a separate machine-readable manifest file

Those belong to the later community plugin/package supply-chain work.

## Managed Layout

Under the configured file root:

- `external-skills-downloads/`
  - raw fetched artifacts
- `external-skills-installed/`
  - one directory per installed skill
- `external-skills-installed/index.json`
  - machine-readable installed-skill registry

Each installed skill entry stores:

- `skill_id`
- `display_name`
- `installed_at_unix`
- `source_kind`
- `source_path`
- `install_path`
- `skill_md_path`
- `sha256`
- `active`

## Config Evolution

Extend `[external_skills]` with:

- `install_root`
  - optional
  - defaults under the configured file root
- `auto_expose_installed`
  - default `true`
  - controls whether installed skills appear in the capability snapshot

Keep existing fields unchanged:

- `enabled`
- `require_download_approval`
- `allowed_domains`
- `blocked_domains`

## Runtime Configuration

`ToolRuntimeConfig` should mirror the new external-skills settings so tool
executors do not need to query environment variables or parse config ad hoc.

## Provider And Prompt Exposure

The provider tool schema remains static, but it gains new built-in functions:

- `external_skills_install`
- `external_skills_list`
- `external_skills_inspect`
- `external_skills_invoke`
- `external_skills_remove`

The capability snapshot should also gain a deterministic section:

- `[available_external_skills]`
- one line per active installed skill:
  `- <skill_id>: <summary>`

This gives the model two discovery channels:

- structured lifecycle tools
- human-readable installed-skill list inside the system prompt

## Dispatch Model

`execute_tool_core_with_config()` stays static at the top level.

The new dispatch is:

- lifecycle tool name is static
- lifecycle tool internally resolves `skill_id`
- lifecycle tool loads managed install state

That avoids dynamic function registration while still making the runtime fully
usable.

## Migration Integration

Migration should remain explicit and non-magical:

- legacy skills are still detected and preserved in `profile_note`
- warnings should say LoongClaw does not auto-install migrated skill runtimes
- migration docs should point to the new install/list/invoke loop as the next
  operator step

This preserves safety and keeps imported metadata auditable.

## Error Handling

Installation errors must be explicit and deterministic:

- source path escapes configured file root
- archive cannot be read
- archive contains no `SKILL.md`
- archive contains multiple candidate roots
- install target already exists without replacement approval
- malformed or empty `SKILL.md`

Invocation errors must distinguish:

- unknown `skill_id`
- inactive skill
- missing `SKILL.md`
- unreadable managed install

Removal errors must distinguish:

- unknown `skill_id`
- managed path missing
- index update failure

## Testing Strategy

### Config And Runtime

- default config includes new external-skills runtime fields
- runtime config mirrors config correctly

### Install Path

- install from directory succeeds
- install from `.tar.gz` succeeds
- missing `SKILL.md` fails
- multiple roots fail deterministically
- duplicate installs require explicit replace behavior

### Registry And Exposure

- tool registry includes new static lifecycle tools
- capability snapshot includes `[available_external_skills]` when enabled
- provider tool definitions include the new lifecycle functions

### Invoke Path

- invoke returns deterministic instruction payload
- invoke rejects missing/inactive skills

### Remove Path

- remove updates index and filesystem state

### Regression

- existing `external_skills.fetch` and `external_skills.policy` tests remain
  green
- migration tests remain green

## Rollout Notes

This slice intentionally stops short of dynamic executable plugins and signed
package supply chain. It provides a complete operator-visible runtime loop for
instruction-style skills while preserving LoongClaw's current static tool
architecture and safety posture.
