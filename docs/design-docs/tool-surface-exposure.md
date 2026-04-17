# Tool Surface Exposure

This document defines how Loong advertises tools to provider-facing models while
preserving kernel-governed execution, approval, audit, and hidden-tool
progressive disclosure.

## Read This Document When

- you are changing which tools appear in provider tool schemas
- you are changing `tool.search` / `tool.invoke` discovery behavior
- you are deciding whether a capability should be direct, hidden, or both
- you are reviewing prompt, gateway, or runtime snapshot changes related to
  tool visibility

## Problem

The runtime already has many precise canonical tools, but provider-facing tool
schemas should stay assistant-first and low-entropy.

A provider surface that exposes only `tool.search` and `tool.invoke` keeps
hidden-tool governance strong, but it also pushes common work through an extra
search round-trip. A provider surface that exposes every canonical tool lowers
that friction, but it overwhelms the model, weakens prompt clarity, and makes
progressive disclosure less meaningful.

Loong needs one design that keeps all of the following true at the same time:

- common tasks trigger quickly
- hidden specialized tools stay governed
- search remains multilingual and metadata-driven
- provider schemas, prompt copy, runtime snapshots, and operator surfaces stay
  aligned

## Design Goals

1. Keep the provider-visible tool surface extremely small.
2. Prefer short action names over taxonomy-heavy names.
3. Keep common file, edit, shell, web, browser, and memory work one call away.
4. Preserve `tool.search -> tool.invoke` for hidden specialized tools.
5. Preserve canonical internal tool identities for governance, telemetry,
   testing, and runtime routing.
6. Route direct tools by payload shape rather than by query hardcoding.

## Three Exposure Layers

### 1. Direct tools

Direct tools are the small provider-visible action surface used for common work.
They must be short, high-prior, and assistant-first.

Current direct tool vocabulary:

- `read`
- `write`
- `exec`
- `web`
- `browser`
- `memory`

A direct tool is a facade. It does not replace the canonical internal tools.
Instead, it dispatches to the canonical tool that matches the payload shape.

Examples:

- `read { path }` -> `file.read`
- `read { query }` -> `content.search`
- `read { pattern }` -> `glob.search`
- `write { path, content }` -> `file.write`
- `write { path, old_string, new_string }` -> `file.edit`
- `web { url }` -> `web.fetch`
- `web { query }` -> `web.search`

If a payload is ambiguous, the facade must fail clearly instead of guessing.

### 2. Discovery gateway

The discovery gateway remains provider-visible:

- `tool.search`
- `tool.invoke`

Use it only when no direct tool fits or when the task needs a hidden
specialized tool.

`tool.search` remains metadata-driven and multilingual. It should not depend on
hardcoded English query phrases.

### 3. Hidden canonical tools

Canonical tools remain the governed execution substrate.
They keep their precise names, schemas, approval behavior, and telemetry.
Examples include:

- `file.read`
- `file.write`
- `file.edit`
- `shell.exec`
- `bash.exec`
- `web.fetch`
- `http.request`
- `browser.open`
- `browser.extract`
- `browser.click`
- `memory_search`
- `memory_get`
- session / approval / delegation / provider / external-skill surfaces

Hidden tools are not advertised directly in provider tool schemas.
They become callable through `tool.invoke` only after discovery returns a valid
lease-bearing tool card.

## Surface Metadata

Every tool belongs to a structured surface such as:

- `read`
- `write`
- `exec`
- `web`
- `browser`
- `memory`
- `approval`
- `session`
- `delegate`
- `provider`
- `external`
- `config`
- `channel`

This metadata is shared across:

- prompt capability snapshots
- `tool.search` results
- conversation advisory rendering
- runtime snapshots
- gateway read models
- status surfaces

The shared metadata keeps prompt guidance, discovery cards, and operator-facing
read models aligned without exposing every canonical tool directly.

## Discovery Policy

Canonical tools that are already covered by a visible direct tool should not be
surfaced by `tool.search` just to recreate the direct path through a lease.
The direct surface should stay the normal path for common work.

`tool.search` should focus on hidden specialized tools such as:

- approval workflows
- session and delegation controls
- provider switching
- config and migration helpers
- external skill management
- lower-level HTTP access
- managed browser companion workflows
- channel-specific operator tools

## Prompt Contract

Prompt copy should teach this order:

1. use a direct tool when it fits
2. use `tool.search` only when no direct tool fits
3. use `tool.invoke` only with a fresh lease from `tool.search`

This keeps the first action concrete while preserving truthful progressive
closure around hidden capabilities.

## Non-goals

- Do not delete canonical hidden tools.
- Do not replace schema-driven routing with query hardcoding.
- Do not bypass kernel approval or audit rules through direct tool aliases.
- Do not make runtime snapshots leak hidden tool ids that are intentionally
  undisclosed to the model.
