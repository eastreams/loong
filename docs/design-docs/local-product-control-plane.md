# Local Product Control Plane

This document defines the localhost-first product control plane exposed by
Loong's daemon-owned gateway.

It is the repository-native contract for contributors who need to understand
how the gateway, control-plane pairing authority, paired-session continuity,
and operator read models fit together without widening into remote or relay
design yet.

## Why This Exists

Loong has multiple runtime entrypoints, but the local product control plane
needs one stable noun that operators and local clients can rely on:

- a stable localhost front door
- one pairing authority
- one paired-session lease authority
- one replay contract for JSON and SSE consumers

Without that, pairing and continuity drift into several partial surfaces that
each appear to work but do not recover consistently across lifecycle changes.

## Current Boundary

The local product control plane is intentionally:

- loopback-first
- bearer-token protected
- grounded in existing control-plane and runtime stores
- explicit about replay continuity

Out of scope for this document:

- remote/public bind expansion
- relay pairing
- internet-facing bootstrap
- a second durable pairing or lease store

## Authority Model

The local product control plane keeps one authority per concern:

| Concern | Authority |
| --- | --- |
| pairing request lifecycle | `ControlPlanePairingRegistry` |
| paired-session lease lifecycle | `ControlPlaneConnectionRegistry` |
| retained event window | `GatewayEventBus` |
| durable runtime continuity snapshot | `gateway/state.rs` runtime-owned snapshot files |
| operator posture views | gateway read-model projection only |

The gateway may expose richer session/bootstrap surfaces, but it must not
replace the pairing or lease authorities with a gateway-local business store.

## Front Door Contract

The gateway front door is the stable localhost control surface.

Bootstrap precedence:

1. explicit CLI `--port`
2. `LOONG_GATEWAY_PORT`
3. config `[gateway].port`
4. built-in default `127.0.0.1:26306`

Discovery precedence:

1. try the stable localhost front door with the local bearer token
2. fall back to persisted owner-state when the front door is unavailable or
   intentionally diverges (for example explicit ephemeral mode)

This keeps one predictable bootstrap noun while still preserving labs/tests and
override-heavy setups.

## Pairing Contract

The gateway now exposes five paired-session surfaces:

- `POST /v1/pairing/start`
- `POST /v1/pairing/complete`
- `GET /v1/pairing/session`
- `GET /v1/pairing/events`
- `GET /v1/pairing/stream`

And the operator pairing inbox remains visible through:

- `GET /v1/pairing/requests`
- `POST /v1/pairing/resolve`

### `pairing/start`

Issues a challenge and returns bootstrap metadata telling local clients how to
continue the flow.

### `pairing/complete`

Consumes the challenge, verifies the device signature, reuses the pairing
registry's decision logic, and issues a paired-session lease on success.

### `pairing/session`

Returns the live paired-session state:

- principal
- lease expiry
- last acknowledged replay position
- replay window metadata
- recovery contract (`fresh`, `resumed`, `stale`)

### `pairing/events`

Returns retained events after a cursor, updates the acknowledged replay
position when requested, and surfaces `stale_cursor` explicitly when the caller
requests a replay position outside the retained window.

### `pairing/stream`

SSE companion to `pairing/events`. It must use the same replay window and stale
cursor semantics as JSON fetch.

## Replay Semantics

The replay contract is intentionally explicit:

- `fresh`: no acknowledged replay position yet
- `resumed`: the retained window can resume from the last acknowledged replay
  point
- `stale`: the last acknowledged replay point fell outside the retained window;
  the client must resume from the earliest resumable boundary

`stale_cursor` is a contract outcome, not a hidden best-effort downgrade.

JSON and SSE consumers must stay aligned on:

- cursor interpretation
- earliest resumable boundary
- stale detection
- acknowledgement behavior

## Durability

Durability is split intentionally:

- pairing/business truth remains in the existing pairing and session stores
- runtime-owned continuity snapshots keep:
  - non-expired paired-session leases
  - acknowledged replay position
  - retained gateway event window

This split allows restart continuity without creating a second business
authority.

## Operator Read Models

Operator surfaces remain projections, not authorities.

The gateway operator summary should expose:

- gateway owner/front-door posture
- channel/runtime posture
- pairing posture
- node inventory posture

Node inventory is a read-model aggregation over:

- approved paired devices
- managed bridge surfaces

This document intentionally keeps node inventory projection-only for now.

## Modifier Rules

When changing this area:

1. keep one pairing authority
2. keep one lease authority
3. keep JSON and SSE replay semantics aligned
4. keep localhost bootstrap stable before widening the trust surface
5. treat operator summaries and node inventory as projections unless a larger
   design explicitly promotes them
