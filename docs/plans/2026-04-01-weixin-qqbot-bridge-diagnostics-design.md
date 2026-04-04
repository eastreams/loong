# Weixin and QQBot Bridge Diagnostics Design

**Scope**

This slice deepens operator diagnostics for the plugin-backed `weixin`,
`qqbot`, and `onebot` channel surfaces that were added in the previous bridge
support work.

The immediate scope is:

- make `loongclaw doctor` truthfully validate plugin-backed bridge surfaces
- avoid false failures when a bridge surface is intentionally owned by an
  external plugin or gateway
- improve onboarding metadata so operators are pointed at the right status
  command
- keep the diagnostic contract registry-first instead of teaching daemon-side
  callers about individual channels

This slice does not add native `weixin` or `qqbot` runtimes, plugin discovery,
or health probing of external gateway processes.

**Problem Statement**

The current bridge-backed surfaces expose inventory and setup metadata, but they
do not participate in operator diagnostics in a truthful way.

The root cause is structural:

- the current doctor pipeline only understands `OperationHealth` and
  `ReadyRuntime`
- `OperationHealth::Unsupported` is globally mapped to `Fail`
- bridge-backed `weixin`, `qqbot`, and `onebot` surfaces intentionally use
  `Unsupported` to mean "LoongClaw reserves the surface contract, but the live
  send or serve runtime belongs to an external plugin"

That means the naive change would be wrong in both directions:

- if we keep `doctor_checks: &[]`, `loongclaw doctor` stays silent about bridge
  surface misconfiguration
- if we attach the existing `OperationHealth` trigger directly, doctor would
  report false failures for correctly configured bridge surfaces

Operators would then see "broken" diagnostics for a surface that is actually
configured exactly as intended.

**Reference Findings**

Recent public PicoClaw channel docs reinforce the same product split that
LoongClaw needs here:

- the Weixin docs separate onboarding, token-based configuration, and gateway
  runtime ownership instead of pretending the local product owns the full login
  flow
- the QQ docs separate credential validation, sandbox constraints, and gateway
  runtime behavior
- the OneBot docs frame the surface as a stable protocol contract in front of
  external runtimes such as NapCat or Go-CQHTTP

The useful takeaway is not to clone those flows. The useful takeaway is that
bridge surfaces need first-class diagnostic language that validates LoongClaw's
own contract without misreporting external runtime ownership as a local fault.

**Chosen Design**

### 1. Add a bridge-specific doctor trigger in the registry contract

LoongClaw should add one new `ChannelDoctorCheckTrigger` variant dedicated to
plugin-backed bridge surfaces.

Recommended name:

- `PluginBridgeHealth`

This trigger still lives beside operation metadata in the channel registry, so
doctor semantics remain attached to the same source of truth as operation
labels, commands, and requirements.

### 2. Keep the existing generic health mapping unchanged

The existing `OperationHealth -> DoctorCheckLevel` mapping should stay as-is for
native and config-backed surfaces.

That preserves current semantics:

- `Ready` -> `Pass`
- `Disabled` -> omitted or `Warn` according to current caller behavior
- `Unsupported` -> `Fail`
- `Misconfigured` -> `Fail`

This avoids leaking plugin-specific meaning into generic operation health.

### 3. Interpret plugin bridge health from shared snapshot facts

The new trigger should use snapshot facts that already exist in the registry
projection:

- `snapshot.compiled`
- `operation.health`
- `operation.detail`
- snapshot notes such as `bridge_runtime_owner=external_plugin`
- snapshot notes such as `selection_error=...`

Recommended semantics:

- `Disabled` -> omit the doctor check, same as current behavior
- `Misconfigured` -> `Fail`
- `Ready` -> `Pass`
- `Unsupported` + `bridge_runtime_owner=external_plugin` + compiled surface ->
  `Pass`
- `Unsupported` + uncompiled surface -> `Fail`
- `Unsupported` without the external-plugin ownership note -> `Fail`

This keeps the rule declarative and generic across `weixin`, `qqbot`, and
`onebot`.

### 4. Emit operation-level bridge contract checks

Doctor should report one check per bridge operation instead of collapsing send
and serve into a single channel-wide status.

That matters because the requirements differ:

- outbound send only needs bridge connectivity and credentials
- inbound serve additionally needs the appropriate allowlist or routing
  constraints

Examples of truthful outcomes:

- `weixin send` can pass while `weixin serve` fails if
  `allowed_contact_ids` is still empty
- `qqbot send` can pass while `qqbot serve` fails if the gateway credentials
  exist but `allowed_peer_ids` are missing

### 5. Point plugin-backed onboarding status to doctor

Once bridge surfaces participate in doctor, their onboarding metadata should
use:

- `status_command = "loongclaw doctor"`

and keep:

- `repair_command = None`

That is the most truthful contract:

- the operator can use doctor to verify LoongClaw's bridge-side configuration
- LoongClaw still does not pretend it can automatically repair an external
  plugin or gateway

**Why This Is The Right Next Step**

This design fixes the real integration gap without creating new architectural
debt:

- registry metadata stays authoritative
- daemon code gains one small new semantic branch instead of channel-specific
  hardcoding
- plugin-backed surfaces become visible in doctor without false red failures
- onboarding guidance becomes consistent with the actual diagnostic entry point

Most importantly, the design preserves a clean separation of concerns:

- LoongClaw validates its bridge-facing contract
- external plugins or gateways still own their own live runtime process
- future native adapters can later switch back to the existing
  `OperationHealth` and `ReadyRuntime` flow without undoing this slice
