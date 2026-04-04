# Channel Bridge Managed Discovery Design

## Scope

This slice deepens LoongClaw's support for plugin-backed `weixin`, `qqbot`,
and `onebot` surfaces by exposing managed bridge-plugin discovery through
operator-facing inventory and doctor flows.

The immediate scope is:

- scan the managed external-skills install root for bridge-plugin manifests
- derive bridge readiness from existing kernel translation logic
- project managed discovery into `ChannelSurface`
- surface managed discovery through `loongclaw channels`, runtime snapshot, and
  `loongclaw doctor`

This slice does not:

- claim to discover every unmanaged plugin on disk
- add a new global plugin manifest schema
- change the meaning of existing operation-level bridge contract checks
- add native `weixin`, `qqbot`, or `onebot` runtimes

## Problem Statement

LoongClaw already models `weixin`, `qqbot`, and `onebot` as plugin-backed
surfaces, and the previous slice made their bridge contract typed and visible.

That still leaves an operator-facing gap:

- `channels` can show the static surface contract, but not whether a managed
  plugin matching that contract is actually present
- `doctor` can show that a configured surface is owned by an external plugin,
  but not whether LoongClaw's managed plugin inventory contains a compatible
  bridge manifest
- a plugin can declare the right `channel_id` while still missing
  `transport_family` or `target_contract`, and operator output currently has no
  place to show that difference

The root cause is that static registry truth and dynamic managed discovery truth
still live on different sides of the architecture, and the dynamic side has not
been projected into channel inventory yet.

## Constraints

The design has to preserve the existing layer split:

- `kernel` owns scanning and bridge translation
- `app` owns channel-registry compatibility and inventory assembly
- `daemon` owns doctor and CLI rendering

It also has to keep two meanings separate:

- existing operation-level bridge checks are about configured surface contract
  and runtime ownership
- new discovery output is about managed plugin inventory under a known root

The design must not collapse those meanings into one doctor check.

## Approaches Considered

### 1. Scan only inside `daemon`

Rejected because it would duplicate scan, translation, and compatibility logic
 inside CLI code.

That would make `doctor` special-case the problem while leaving `channels` and
runtime snapshot behind.

### 2. Add a new dedicated operator `plugin_roots` config now

Rejected for this slice because it would widen the change far beyond the core
operator gap.

An explicit new config surface may still be justified later, but the smallest
correct step today is to expose managed discovery from the existing
operator-owned install root.

### 3. Add managed discovery to `ChannelSurface`

Chosen because it keeps static and dynamic facts separate while letting every
operator-facing flow reuse the same typed summary.

This path can:

- reuse `kernel::PluginScanner` and `kernel::PluginTranslator`
- reuse the app-owned channel registry validator
- enrich `channels`, runtime snapshot, and `doctor` from one inventory seam
- keep discovery explicitly labeled as managed install-root discovery

## Chosen Design

### 1. Model managed discovery on `ChannelSurface`

Add a typed managed-discovery payload for plugin-backed surfaces only.

The payload should answer:

- was managed discovery configured
- did the scan succeed
- how many compatible, incomplete, and incompatible manifests were found
- which discovered plugins matched the surface

This belongs on `ChannelSurface`, not `ChannelCatalogEntry`, because discovery
is environment-dependent rather than registry-static.

### 2. Use the managed external-skills install root as the discovery source

This slice will scan `external_skills.install_root` when it is configured.

That choice is intentionally narrow:

- it is already an operator-owned filesystem root
- it avoids importing runner-only spec scan roots into daemon flows
- it lets output stay truthful by calling the result managed discovery

If the install root is absent, discovery is reported as unavailable instead of
pretending that no plugin exists anywhere.

### 3. Combine kernel translation with app compatibility

Each discovered manifest should be evaluated through two lenses:

- `kernel` translation provides runtime profile and bridge-contract readiness
  such as `transport_family`, `target_contract`, `account_scope`, and missing
  fields
- `app` registry validation provides channel-surface compatibility such as
  missing `setup.surface` or unsupported channel ownership

That combination avoids the false-positive case where a manifest looks
compatible by `channel_id` and `setup.surface` alone but is still missing
required bridge metadata.

### 4. Preserve current doctor check semantics

Existing operation-level checks remain unchanged:

- they still describe configured surface contract truth
- they still treat external bridge runtime ownership as the reason an
  unsupported operation can be acceptable

New managed-discovery checks are added alongside them at the surface level.

That keeps old tests and operator meaning stable while adding the missing
inventory signal.

### 5. Let `channels` and runtime snapshot inherit the new summary

Because `channels` and runtime snapshot already serialize `ChannelInventory`,
they should automatically expose the new managed-discovery payload once
`ChannelSurface` carries it.

Text rendering should add a compact managed-discovery line and per-plugin detail
lines for matched manifests.

## Why This Design

This is the smallest correct next step because it closes the missing operator
loop without inventing a new policy system:

- `kernel` still owns scanning and readiness derivation
- `app` still owns channel compatibility and inventory grouping
- `daemon` still owns presentation and doctor policy

It also stays truthful about scope:

- discovery is explicitly managed-root discovery
- existing contract checks keep their original meaning
- incomplete manifests are not reported as ready-compatible

That gives operators a materially better view of `weixin`, `qqbot`, and
`onebot` bridge support without expanding the problem into a larger plugin
configuration redesign.
