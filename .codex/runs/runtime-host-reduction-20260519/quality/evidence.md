# Runtime Host Reduction Evidence

## Lane

- Pass 1, Lane 1: Runtime Host Reduction
- Worktree: `/Users/chum/worktrees/loong/release-hardening-runtime-host-20260519`
- Re-grounded live seam: `crates/daemon/src/runtime_protocol_host.rs`

## Re-grounding Note

- The redesign dossier still named `crates/app/src/runtime_protocol_host.rs`, but the
  merged `origin/dev` tree no longer contains that file.
- On the live baseline `origin/dev @ e6880c91583ad61ec94927d81a92b87ae01f99c8`,
  the strongest surviving main-path runtime host bridge was the daemon-local
  adapter `crates/daemon/src/runtime_protocol_host.rs`.

## Code Evidence

- Deleted `crates/daemon/src/runtime_protocol_host.rs`.
- Replaced the daemon-local `AppProtocolRuntimeHost` adapter flow in:
  - `crates/daemon/src/turn_cli.rs`
  - `crates/daemon/src/gateway/api_turn.rs`
  - `crates/daemon/src/control_plane_server/turn.rs`
- New main-path shape:
  - CLI `ask` and `turn run` call `loong_app::chat::{run_cli_ask, run_cli_chat}` directly.
  - Gateway and control-plane routed turns call
    `loong_app::turn_gateway::{build_turn_gateway_request, run_turn_gateway}` directly.
- The post-redesign spine stays intact: execution still routes through `loong_app`
  runtime services rather than reintroducing daemon-side legacy helpers.

## Before/After Line Counts

- `crates/daemon/src/runtime_protocol_host.rs`: `240` -> deleted
- `crates/daemon/src/turn_cli.rs`: `239` -> `132`
- `crates/daemon/src/gateway/api_turn.rs`: `236` -> `241`
- `crates/daemon/src/control_plane_server/turn.rs`: `240` -> `241`
- Net lane diff across touched runtime-host files: `955` -> `614` (`-341` lines)

## Focused Verification

- `cargo test -p loong --no-default-features --features mvp,test-support gateway_turn_returns_service_unavailable_without_acp_backend`
  - passed
- `cargo test -p loong --no-default-features --features mvp,test-support gateway_run_turn_persists_acp_session_metadata_into_configured_sqlite_store`
  - passed
- `cargo test -p loong --no-default-features --features mvp,test-support gateway_acp_operator_endpoints_surface_shared_session_truth`
  - passed
- `cargo test -p loong --no-default-features --features mvp,test-support turn_submit_and_result_fetch_complete_with_streamed_backend`
  - passed
- `cargo test -p loong --no-default-features --features mvp,test-support turn_run_cli_latest_session_selector_process_uses_selected_root_session_history`
  - passed
- `cargo test -p loong --no-default-features --features mvp,test-support ask_cli_accepts_latest_session_selector`
  - passed
- `./scripts/check_architecture_boundaries.sh`
  - passed
- `./scripts/cargo-local-toolchain.sh fmt --all -- --check`
  - rerun after formatting before closeout

## Constraint Check

- No daemon-side legacy execution helper was reintroduced.
- No UI/provider/channel/config concerns were pushed into `loong-core`.
- No forbidden crate-edge change was introduced; the cleanup avoided moving the
  host into `loong-app`, which would have violated the existing DAG by adding a
  `loong-app -> loong-app-protocol` dependency.

## Residual Note

- The daemon-local runtime host bridge is gone.
- Residual runtime-host-style projection still exists as thin address/request
  assembly in `gateway/api_turn.rs` and `control_plane_server/turn.rs`, but it
  now terminates directly at shared `loong_app` runtime services instead of a
  separate production host adapter.
- `loong-app-protocol` remains in the tree for broader protocol surfaces, but it
  no longer owns these daemon main-path runtime turn entrypoints.

## Commit Shape

- Intended claim 1: delete the daemon runtime protocol host adapter and route
  main-path turn surfaces directly into shared `loong_app` runtime services.
- Intended claim 2: record lane evidence and verification.
