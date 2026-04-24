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
- search remains prompt-driven and metadata-driven
- provider schemas, prompt copy, runtime snapshots, and operator surfaces stay
  aligned
- precise file mutation is one call away without overloading `write`
- retryable tool failures can continue through the tool loop instead of collapsing into a text-only fallback

## Design Goals

1. Keep the provider-visible tool surface extremely small.
2. Prefer short action names over taxonomy-heavy names.
3. Keep common file, grep, find, edit, shell, web, browser, and memory work one call away.
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
- `grep`
- `find`
- `edit`
- `write`
- `exec`
- `web`
- `browser`
- `memory`

A direct tool is a facade. It does not replace the canonical internal tools.
Instead, it dispatches to the canonical tool that matches the payload shape.

Examples:

- `read { path }` -> `file.read`
- `read { path, offset, limit }` -> `file.read`
- `grep { query }` -> `content.search`
- `read { query }` -> `content.search` (compatibility path; prefer `grep` for direct text search)
- `find { pattern }` -> `glob.search`
- `read { pattern }` -> `glob.search` (compatibility path; prefer `find` for direct path matching)
- `edit { path, edits }` -> `file.edit`
- `edit { path, old_string, new_string }` -> `file.edit` (legacy exact-edit mode)
- `write { path, content }` -> `file.write`
- `exec { command }` -> `shell.exec`
- `exec { script }` -> `bash.exec`

Exec results keep a stable structured payload. Inline `stdout` / `stderr` previews remain easy to read, while `details.stdout` / `details.stderr` report truncation metadata and expose any saved `full_output_path` handoff targets. When output is truncated, callers should first try `details.handoff.recommended_payload` with `read`; `details.handoff.recipes.<stream>.*` then exposes alternate first-page / last-page / wider-byte windows without forcing the model to invent paging arguments.
- `web { url }` -> `web.fetch`
- `web { query }` -> `web.search`
- `browser { url }` -> `browser.open` or managed browser session start, depending on payload shape
- `browser { session_id, mode }` -> `browser.extract` or managed browser snapshot, depending on payload shape
- `browser { session_id, selector }` -> managed browser click
- `browser { session_id, selector, text }` -> managed browser type
- `browser { session_id, condition }` -> managed browser wait

Only `web { query }` depends on configured web-search providers. `web { url }`, low-level HTTP request mode, browser sessions, and other networked tools remain ordinary network paths with their own runtime policy.

Operator-facing runtime surfaces should expose that split directly instead of collapsing it into one vague "web enabled" bit. Status views, gateway summaries, and runtime snapshots should separately report ordinary network access, query-search availability, the default search provider, and whether that provider is credential-ready. Runtime snapshot text / JSON should also carry an explicit `web_access` summary instead of forcing operators to infer that boundary from `web_fetch` and `web_search` blocks alone.

The browser surface owns the managed-browser namespace. Provider-facing guidance should teach `browser`, not a long tail of `browser.companion.*` names or an exposed sub-action enum.

If a payload is ambiguous, the facade must fail clearly instead of guessing.

### 2. Discovery gateway

The discovery gateway remains provider-visible:

- `tool.search`
- `tool.invoke`

Use it only when no direct tool fits or when the task needs a hidden
specialized tool.

`tool.search` should stay metadata-driven, but the guidance should come from
short prompt snippets, clean tool names, and schema terms rather than from a
large pile of hardcoded query examples.

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
- agent-control / capability-expansion / channel surfaces

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
- `agent`
- `skills`
- `channel`

The current simplification direction is:

- keep core runtime-control work grouped under `agent`
- keep capability-expansion work grouped under `skills`
- keep channel-specific tools separate from core surfaces

That last point is intentional. Channel surfaces such as Feishu are product add-ons,
not part of the core tool vocabulary, and should stay structurally separable from
Loong core as channel crates continue to split out over time.

This metadata is shared across:

- short prompt snippets for each visible tool or hidden surface
- tool-specific guidance bullets similar in spirit to pi's `promptSnippet` /
  `promptGuidelines` split
- grouped hidden-surface discovery cards (`agent`, `skills`)
- prompt capability snapshots
- `tool.search` results
- conversation advisory rendering
- followup request summaries and reduced result envelopes
- approval request summaries/details (with visible tool names plus sanitized request summaries)
- runtime snapshots
- gateway read models
- status surfaces

For file mutation, the prompt contract should now distinguish:

- `edit` for surgical exact-match replacements in existing files
- `write` for new files and whole-file replacement

The capability snapshot should append active surface-specific guidance bullets only
for the surfaces that are actually visible in the current runtime. That keeps the
system prompt short while still teaching high-frequency behaviors like:

- read before shell for normal inspection
- `offset` / `limit` paging for large files
- `edits` for surgical exact replacements
- `script` for shell-heavy exec tasks

The shared metadata keeps prompt guidance, discovery cards, followup/approval
surfaces, and operator-facing read models aligned without exposing every
canonical tool directly. When exact governance state still needs the canonical
name, user-facing payloads should pair it with a visible counterpart instead of
teaching only the internal identifier.

This applies to operator/runtime payloads too: session tool-policy status,
background-task summaries, and approval queues should preserve raw canonical ids
for replay/governance while also surfacing visible names such as `read`, `exec`,
or `browser` for the primary user-facing summary. For task/tool-policy payloads,
carry both raw and visible forms for the requested and effective tool sets instead
of forcing operators to infer one from the other.

## Discovery Policy

Canonical tools that are already covered by a visible direct tool should not be
surfaced by `tool.search` just to recreate the direct path through a lease.
The direct surface should stay the normal path for common work.

`tool.search` should focus on hidden specialized tools such as:

- agent runtime and control work (approvals, sessions, delegation, provider/config state)
- external skill management
- lower-level HTTP access
- channel-specific operator tools

When the runtime advertises external skills to the model, the advertised set must
already reflect invocation truth: hide manual-only skills and other runtime-
ineligible skills from model-facing catalogs, lists, and inspect paths instead of
teaching the model to invoke them and then rejecting the call later.

Managed browser workflows stay under the direct `browser` surface for normal model-facing use, even when the canonical implementation routes into internal `browser.companion.*` tools.

Hidden search results should prefer surface-level ids instead of spraying many
per-operation names into the model context. Today that means grouped results
such as `agent`, `skills`, and `channel`, while direct surfaces like `browser`
or `web` remain the normal path for common work.

## Prompt Contract

Prompt copy should teach this order, using short prompt snippets instead of
example-query catalogs:

1. use a direct tool when it fits
2. use `tool.search` only when no direct tool fits
3. use `tool.invoke` only with a fresh lease from `tool.search`

This keeps the first action concrete while preserving truthful progressive
closure around hidden capabilities.

## Non-goals

- Do not delete canonical hidden tools.
- Do not replace schema-driven routing with query hardcoding.
- Do not stuff multilingual query examples into the codebase just to teach tool
  use.
- Do not bypass kernel approval or audit rules through direct tool aliases.
- Do not make runtime snapshots or search cards leak a long tail of hidden
  per-operation ids when one surface-level id is enough.
