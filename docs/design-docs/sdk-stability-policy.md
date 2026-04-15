# SDK Stability Policy

## Purpose

This document defines which SDK-adjacent surfaces should be treated as:

- stable
- additive
- experimental
- internal

Its purpose is to prevent Loong from making accidental compatibility
promises in the wrong layers.

## Core Rule

Loong should stabilize artifact and workflow contracts before stabilizing
internal helper APIs.

The default order should be:

1. product vocabulary
2. package and artifact contracts
3. validator and projection semantics
4. promotion artifact schemas
5. only later, selective helper APIs if they prove durable

## Stability Matrix

| Surface | Recommended Stability |
|---------|------------------------|
| `product mode` vocabulary and operator-facing meaning | Stable |
| high-level blocked-reason families | Additive |
| internal integration SDK descriptor layout | Internal |
| internal registry implementation details | Internal |
| package manifest shapes | Additive moving toward Stable |
| skill package layout | Additive moving toward Stable |
| setup metadata semantics | Additive moving toward Stable |
| ownership semantics | Additive moving toward Stable |
| validator meaning | Additive |
| controlled runtime-lane contract | Additive |
| trust and provenance field shape | Additive |
| runtime-capability candidate schema | Additive |
| promotion plan taxonomy | Additive |
| live promotion executors | Experimental |
| learning or ranking layers | Experimental |
| internal convenience helpers in `crates/app` | Internal |

## Practical Reading

- internal maintainer seams should stay movable
- public artifact contracts should become predictable first
- promotion should stabilize artifact meaning before executor automation
