# Local Product Control Plane

## Goal

Expose one stable localhost control surface for operators and local paired
clients so pairing, paired-session continuity, replay recovery, and operator
status all route through a consistent product contract.

## In Scope

- stable localhost gateway front door
- local bearer-token bootstrap
- pairing bootstrap and approval flow
- paired-session lease issuance
- paired-session JSON replay and SSE stream recovery
- operator summary and node/trust posture projection

## Out of Scope

- remote/public exposure
- relay pairing
- browser/public onboarding beyond localhost
- second durable pairing/session authority

## User Stories

### Operator

- As an operator, I can inspect gateway ownership and pairing posture from one
  local surface.
- As an operator, I can approve or reject pairing requests without leaving the
  gateway contract.
- As an operator, I can understand whether the current control surface came
  from the default front door, config, env, or CLI override.

### Local Paired Client

- As a local client, I can discover the gateway through one stable localhost
  front door before falling back to owner-state.
- As a local client, I can start pairing, complete pairing, obtain a paired
  session lease, and use that lease for session inspection and replay.
- As a local client, I get explicit replay recovery signals instead of silent
  truncation when my cursor is stale.

## Functional Contract

### Front Door

- default host: `127.0.0.1`
- default port: `26306`
- precedence: CLI > env > config > default
- explicit ephemeral mode remains available through `--port 0`

### Pairing Bootstrap

- `POST /v1/pairing/start`
  - returns a challenge and bootstrap metadata
- `POST /v1/pairing/complete`
  - requires device identity + signature + challenge
  - reuses the existing pairing registry
  - returns a paired-session lease on success

### Paired Session

- `GET /v1/pairing/session`
  - returns session principal, expiry, replay posture, and replay window
- `GET /v1/pairing/events`
  - returns retained events after a cursor
  - optionally updates the acknowledged replay position
- `GET /v1/pairing/stream`
  - SSE form of the same replay contract

### Pairing Inbox

- `GET /v1/pairing/requests`
- `POST /v1/pairing/resolve`

These remain the operator-facing pairing review and resolution surfaces.

## Replay Recovery Rules

The contract must expose:

- `fresh`
- `resumed`
- `stale_cursor`
- `earliest_resumable_after_seq`

Clients must never be forced to infer replay safety from missing rows or
truncated SSE output.

## Product Constraints

- Pairing and lease authority remain on existing control-plane primitives.
- Operator summaries and node inventory are projections only.
- Restart continuity may persist runtime-owned replay state, but that does not
  create a second business database.

## Acceptance Signals

- operators can complete pairing and inspect status entirely through the local
  gateway surface
- local clients can bootstrap, pair, inspect, replay, and stream through one
  paired-session contract
- replay errors are explicit and actionable
- restart continuity preserves valid leases and replay posture
- operator summary reflects front-door, pairing, and node/trust posture without
  inventing new authority
