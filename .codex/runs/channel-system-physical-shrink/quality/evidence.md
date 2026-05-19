# Channel System Physical Shrink Evidence

## Scope

- Lane: `Pass 1 / Lane 2 - Channel System Physical Shrink`
- Worktree: `/Users/chum/worktrees/loong/release-hardening-channel-system-20260519`
- In-scope hotspots:
  - `crates/app/src/channel/registry.rs`
  - `crates/app/src/channel/sdk.rs`
  - `crates/app/src/channel/dispatch.rs`
  - tightly related channel support modules only

## Code Evidence

- Command/account-resolution and session-send ownership moved out of the dispatch hotspot into:
  - `crates/app/src/channel/commands/accounts.rs`
  - `crates/app/src/channel/commands/session_send.rs`
- Runtime-backed channel registry/status ownership moved out of the registry hotspot into:
  - `crates/app/src/channel/registry/runtime_backed.rs`
- Config-backed channel registry/status ownership moved out of the registry hotspot into:
  - `crates/app/src/channel/registry/config_backed.rs`
- Large inline registry test mass moved out of the production file into:
  - `crates/app/src/channel/registry/core_tests.rs`
- Runtime-backed channel catalog/command-family metadata for Telegram, Feishu, QQBot, Matrix, and WeCom was further moved out of the root registry module into:
  - `crates/app/src/channel/registry/runtime_backed.rs`

## Before / After Line Counts

- `crates/app/src/channel/registry.rs`: `9274 -> 2288`
- `crates/app/src/channel/dispatch.rs`: `3323 -> 2134`
- `crates/app/src/channel/sdk.rs`: `1442 -> 1442`

## Verification

- `./scripts/cargo-local-toolchain.sh check -p loong-app --lib`
  - passed
- `./scripts/cargo-local-toolchain.sh clippy -p loong-app --lib --tests -- -D warnings`
  - passed
- `./scripts/cargo-local-toolchain.sh test -p loong-app channel::registry --lib`
  - passed
  - `101 passed; 0 failed`
- `./scripts/cargo-local-toolchain.sh test -p loong-app channel:: --lib`
  - passed
  - `567 passed; 0 failed`
- `./scripts/check_architecture_boundaries.sh`
  - passed
  - `channel_registry` reported `2892` lines and `HEALTHY`
- Additional bounded extraction verification:
  - `./scripts/cargo-local-toolchain.sh check -p loong-app --lib`
  - `./scripts/cargo-local-toolchain.sh clippy -p loong-app --lib --tests -- -D warnings`
  - `./scripts/cargo-local-toolchain.sh test -p loong-app channel::registry --lib`
  - all passed after the runtime-backed catalog ownership move

## Residual Ownership Notes

- `registry.rs` is now below the hard line cap, but `channel/config/channels.rs` remains a separate watch-pressure hotspot outside this lane.
- `sdk.rs` remained under the hard cap throughout this lane and did not require a structural split in this pass.
- Plugin-bridge catalog truth remains dominant ownership; the new channel registry modules only deepen built-in status assembly and dispatch locality.
- The root registry module now owns less runtime-backed channel catalog truth; the remaining root mass is mostly shared channel catalog metadata and generic registry assembly.

## Changed Files

- `crates/app/src/channel/commands/mod.rs`
- `crates/app/src/channel/commands/accounts.rs`
- `crates/app/src/channel/commands/session_send.rs`
- `crates/app/src/channel/dispatch.rs`
- `crates/app/src/channel/gateway_ingress.rs`
- `crates/app/src/channel/mod.rs`
- `crates/app/src/channel/qqbot/mod.rs`
- `crates/app/src/channel/registry.rs`
- `crates/app/src/channel/registry/descriptors.rs`
- `crates/app/src/channel/registry/config_backed.rs`
- `crates/app/src/channel/registry/core_tests.rs`
- `crates/app/src/channel/registry/runtime_backed.rs`
- `crates/app/src/channel/registry_nostr_impl.rs`
- `crates/app/src/channel/whatsapp/mod.rs`

## Commit-Shape Note

- Implementation commit:
  - `6b1e0ba2 refactor(channel): shrink registry and dispatch ownership`
- Additional bounded ownership-reduction commit:
  - pending in worktree during evidence refresh for the runtime-backed catalog metadata move
- The architectural claim is single-lane only:
  - channel system physical shrink
  - ownership deepening after the plugin-bridge shift
