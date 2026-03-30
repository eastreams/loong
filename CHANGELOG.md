# Changelog

All notable changes to this project will be documented in this file.

The format follows Keep a Changelog and semantic versioning intent.

## [Unreleased]

## [0.1.0] - 2026-03-29

### Added

- Added the first stable LoongClaw baseline after the `0.1.0-alpha.*` prerelease line, carrying forward the current `dev` slice for promotion into `main`.
- Added Feishu channel delivery follow-through including Messaging API integration plus bitable list, create, and search support.
- Added background task CLI flows, discovery-first external skills guidance, doctor security audit coverage, and session-scoped tool consent handling for the daemon/runtime surface.

### Changed

- Changed the default CLI command path to `loong`, expanded channel send surfaces and trust enforcement, and continued hardening browser, approval, consent, plugin, and outbound HTTP governance boundaries.
- Refined release governance, architecture drift evidence, and promotion-readiness checks so the stable line can be promoted from `dev` into `main` with the same strict local gates used in prerelease validation.

### Fixed

- Restored provider-side tool execution when OpenAI-compatible responses emit standalone JSON tool blocks or Ollama-style `<tool_call>...</tool_call>` fallbacks instead of native `tool_calls`.

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

- Introduced the fresh `0.1.0-alpha.1` prerelease line for LoongClaw as a secure Rust foundation for vertical AI agents.
- Preserved the baseline CLI path around guided onboarding, ask or chat flows, doctor repair, and multi-surface delivery for early team evaluation.

### Changed

- Reset canonical release history on `dev` to the new prerelease baseline after invalidating the earlier tracked `0.1.x` release line.
- Made release governance prerelease-aware and seeded contributor notes from the current source snapshot instead of inheriting the invalidated prior tag range.
