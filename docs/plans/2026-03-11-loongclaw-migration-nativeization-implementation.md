# LoongClaw Migration Nativeization Implementation

Date: 2026-03-11
Status: Implemented in `alpha-test` worktree

## Goal

Ship the first real LoongClaw importer that can nativeize prompt/identity
content from other claw-family workspaces into a valid LoongClaw config.

## Implemented Surface

### App Layer

Added `crates/app/src/migration/mod.rs` with:

- `LegacyClawSource`
- `ImportPlan`
- `plan_import_from_path(...)`
- `apply_import_plan(...)`

The app-layer importer:

- scans legacy workspace/config roots plus `workspace/` when present
- reads portable migration files
- auto-detects likely source claw when no explicit hint is provided
- classifies imported content into prompt addendum vs profile note
- ignores stock upstream templates
- rewrites upstream claw self-branding to `LoongClaw`
- parses AIEOS `identity.json` into durable profile-note bullets

### Daemon Layer

Added `crates/daemon/src/migrate_cli.rs` and wired a new subcommand:

```text
loongclawd migrate --input <path> [--output <config>] [--source <auto|nanobot|openclaw|picoclaw|zeroclaw|nanoclaw>] [--force]
```

Behavior:

- reads migration input from a file or directory
- optionally honors explicit `--source`
- loads existing target config when overwriting with `--force`
- applies LoongClaw-native prompt pack + imported overlays
- writes target config
- bootstraps sqlite memory when enabled
- prints a concise import summary and warnings

### Agent Tool Layer

Added an app-native tool in `crates/app/src/tools/claw_migrate.rs` and registered:

- canonical tool name: `claw.migrate`
- provider function alias: `claw_migrate`

Behavior:

- agents can run `mode = "plan"` to preview nativeized migration output
- agents can run `mode = "apply"` to write a target LoongClaw config
- output is structured JSON, suitable for function-calling loops
- imported source branding is normalized to `LoongClaw`
- when `LOONGCLAW_FILE_ROOT` is configured, import input/output paths are sandboxed to that root

### Spec / Hot Handling Layer

Extended spec runtime so hot-routed agents can access the same capability without
duplicating migration logic:

- `CoreToolRuntime` delegates `claw.migrate` requests into the app-native tool
- added tool extension wrapper `claw-migration`
- registered hot example spec: `examples/spec/claw-import-hotplug.json`

This keeps one migration engine while exposing both direct agent tools and
spec-runtime hot handling.

## Mapping Rules In Code

### Prompt Addendum

Imported into `config.cli.system_prompt_addendum`:

- `AGENTS.md`
- `SOUL.md`
- `TOOLS.md`
- `BOOTSTRAP.md`
- `CLAUDE.md`

### Durable Identity / Preferences

Imported into `config.memory.profile_note`:

- `IDENTITY.md`
- `USER.md`
- `MEMORY.md`
- `memory/MEMORY.md`
- AIEOS `identity.json`

### Runtime Defaults Forced By Import

The importer always activates:

- `cli.prompt_pack_id = "loongclaw-core-v1"`
- `memory.profile = "profile_plus_window"`

This is intentional. Migration output should land on the LoongClaw-native
prompt and memory path immediately.

## Known Limits In v0.1

- `HEARTBEAT.md` is only warned about when it contains active tasks.
- skill/runtime-specific content is not deeply translated.
- credentials and auth profiles are not imported.
- source detection is heuristic rather than schema-perfect.
- AIEOS import currently extracts the most useful human-readable fields rather
  than preserving full structured identity.

These limits are acceptable for the first release because the primary user pain
is identity/tuning re-entry, not full operational parity.

## Test Coverage Added

### App Tests

- stock NanoBot templates nativeize to LoongClaw defaults
- custom prompt + identity content survive as addendum/profile note
- ZeroClaw AIEOS identity becomes LoongClaw-flavored profile note
- `claw.migrate` plan mode returns a structured nativeized preview
- `claw.migrate` apply mode writes a target config

### Daemon Tests

- source parser accepts supported source IDs
- `run_migrate_cli(...)` writes a nativeized LoongClaw config
- spec tool-core path can execute `claw.migrate`
- spec tool-extension path can hot-handle `claw-migration`

## Recommended Next Steps

1. Integrate importer into `onboard` as an optional first-run path.
2. Expand config parsing for OpenClaw/PicoClaw inline identity fields.
3. Add an import report that enumerates:
   - stock files replaced
   - custom files preserved
   - warnings requiring manual follow-up
4. Connect future pluggable memory backends to the same imported
   `profile_note` abstraction so migration output stays backend-agnostic.
