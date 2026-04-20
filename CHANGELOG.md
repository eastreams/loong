# Changelog

All notable changes to this project will be documented in this file.

The format follows Keep a Changelog and semantic versioning intent.

## [Unreleased]

## [0.1.2-alpha.1] - 2026-04-20

### Added

- Added mailbox-backed delegate result delivery and wakeups, persisted frozen delegate results, and parent-session delegate result announcements.
- Added workflow-aware daemon sessions shell coverage, latest-session selector support, session search / trajectory artifact CLIs, and richer session inspection surfaces.
- Added provenance-aware workspace recall, registered pre-assembly memory systems, typed memory stage contracts, staged hydration envelopes, compact-stage seams, and narrower cross-session recall artifacts.
- Added runtime-capability family readiness indexes, promotion planning, governed apply surfaces, and delta-evidence capture for release and operator workflows.
- Added gateway localhost control surfaces including health, events, turn endpoints, local clients, event buses, read models, loopback control-plane subscriptions, and remote control-plane hardening.
- Added manifest-first plugin packaging, plugin inventory / doctor CLIs, typed setup metadata and provenance surfaces, plugin setup-readiness gating, managed bridge discovery, stable-target contracts, and governed plugin bridge handoff flows.
- Added config-backed outbound surfaces for webhook, Teams, iMessage, email, IRC, Nostr, Twitch, Tlon, WhatsApp, Matrix, WeCom, and wider runtime-backed multi-channel supervision.
- Added Feishu document, bitable, calendar, websocket, wildcard allowlist, QR onboarding, and emoji reply capabilities.
- Added Xiaomi and Bailian Coding provider support, multi-provider web-search selection, Firecrawl web search provider support, multi-region onboarding endpoint selection, and stronger provider descriptor projections.
- Added bash.exec support with AST-governed prefix rules, file.edit text replacement support, tool concurrency metadata, tool execution timeouts, structured developer tracing, bundled skill packs, and expanded external skill intake safety gates.
- Added typed subagent handles, private operator runtime seams, prompt-orchestrator tool discovery context, prompt personality expansion, durable workspace memory tools, and runtime-self continuity loading.
- Added background tasks CLI, personalize operator preferences, doctor security audit surfaces, audit inspection CLIs, shell completion generation, Android Termux release builds, Linux musl release contracts, and Windows CI coverage.

### Changed

- Reworked memory continuity so durable recall, staged retrieval, runtime-self continuity, workspace corpus handling, compaction, and recall metadata stay aligned across chat, tools, and daemon flows.
- Tightened approval, consent, binding-first runtime seams, governed workflow contracts, session tool-policy controls, trust-event projection, and operator approval replay behavior.
- Productized delegate child orchestration, continuity compaction surfaces, live chat status rendering, fast-lane diagnostics, direct-tool summaries, and shell discoverability while preserving bounded runtime behavior.
- Matured onboarding UX with clearer provider / web-search guidance, safer credential handling, explicit contract labels, profile-aware defaults, and stronger recovery prompts.
- Refined plugin, channel, daemon, conversation, runtime, and memory module boundaries to stay within architecture budgets without splitting the shipped `loong` product surface.
- Refreshed release governance with stricter docs gates, architecture-drift freshness checks, crates.io publish preparation, and stable self-update routing toward the intended release lane.
- Continued the public Loong rebrand across documentation, site navigation, public guides, README flows, and contributor-facing references.

### Fixed

- Restored provider-side tool execution when OpenAI-compatible responses emit standalone JSON tool blocks or Ollama-style `<tool_call>...</tool_call>` fallbacks instead of native `tool_calls`.
- Fixed numerous provider onboarding and auth regressions including auth-profile fallback, missing managed credentials, alternative auth guidance, canonicalized env bindings, quieter auto-model failover logs, and corrected StepPlan endpoint guidance.
- Fixed Feishu reply ordering, websocket TLS/provider setup, document content preservation, card schema handling, callback approval routing, auth permission guidance, and broader tool/runtime regressions.
- Fixed browser companion probe retries, gateway stack-overflow follow-ups, MCP snapshot redaction and proxy safety edges, and multi-channel supervisor shutdown / fail-closed behavior.
- Fixed Windows-specific sqlite, temp-path, UNC-path, worktree cleanup, and locked-file edge cases plus cwd/process-global drift that destabilized tests and runtime cleanup.
- Fixed bash governance matching, broken-rule visibility, rules-dir semantics, timeout cleanup, discoverability regressions, approval routing, and shell summary redaction.
- Fixed session continuity, tool discovery prompt boundaries, continuity fallback, durable flush claims, provider route diagnostics, session policy context handling, and latest-runtime narrowing guards.
- Fixed plugin governance compatibility, setup-readiness validation, manifest write safety, managed discovery rollback, and external-skill policy probing.
- Fixed outbound HTTP trust enforcement, browser boundary handling, webhook auth/status redaction, channel validation, WhatsApp/Twitch/Nostr/IRC serve/runtime regressions, and truthful bridge-backed diagnostics.
- Fixed release, CI, and docs drift issues across architecture snapshots, release artifact checks, arm64 packaging, dependency PR blocking, and publish-flow verification.

## [0.1.0-alpha.2] - 2026-03-19

### Added

- Added a fast-lane summary command for chat flows to surface concise delegate context faster.
- Surfaced the delegate child runtime contract in the app runtime so downstream tooling can reason about effective delegation behavior.

### Changed

- Tightened delegate prompt summary visibility and aligned the effective runtime contract with stricter disabled-tool coverage.
- Hardened the dev-to-main release promotion lifecycle and source enforcement in CI.
- Expanded delegate runtime, private-host, and process stdio test coverage to stabilize the prerelease line before broader promotion.
- Refreshed contributor governance and README visuals, including new Chinese SVG diagrams and restored core harness docs changes.

## [0.1.0-alpha.1] - 2026-03-17

### Added

- Introduced the fresh `0.1.0-alpha.1` prerelease line for Loong as a secure Rust base for vertical AI agents.
- Preserved the baseline CLI path around guided onboarding, ask or chat flows, doctor repair, and multi-surface delivery for early team evaluation.

### Changed

- Reset canonical release history on `dev` to the new prerelease baseline after invalidating the earlier tracked `0.1.x` release line.
- Made release governance prerelease-aware and seeded contributor notes from the current source snapshot instead of inheriting the invalidated prior tag range.
