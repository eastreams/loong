# Changelog

All notable changes to this project will be documented in this file.

The format follows Keep a Changelog and semantic versioning intent.

## [Unreleased]

## [0.1.3] - 2026-03-17

### Added

- Added runtime experiment record, compare, and restore operator workflows, including recorded snapshot reuse and stage restore support.
- Added browser companion preview, diagnostics, and governed install/readiness foundations for first-run flows.

### Changed

- Tightened onboarding and first-run behavior to preserve explicit provider and model choices, improve exit guidance, and align provider binding defaults with the active runtime.
- Compacted `shell.exec`, `file.read`, and `tool.search` follow-up payloads while preserving diagnostics and user-facing field semantics.

### Fixed

- Stabilized browser companion spawn and readiness handling with busy-executable retries, runtime policy alignment, and CI assertion hardening.
- Hardened migrate rollback, runtime restore safety, release-install handoff behavior, and shell follow-up reducer edge cases.

## [0.1.2] - 2026-03-09

### Changed

- Defaulted tool-turn fallback responses to natural-language output when no explicit structured tool response is produced.
- Hardened workspace lint configuration and release-trace linkage checks in release documentation.

## [0.1.1] - 2026-03-09

### Changed

- Release governance upgraded with rich canonical reports, local AI-debug release logs, and automated post-release report generation.

## [0.1.0] - 2026-03-09

### Added

- OSS foundation automation and governance baseline (CI, security automation, release workflow, and policy docs).
