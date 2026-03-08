# Versioning and MSRV Policy

## Versioning

LoongClaw follows semantic versioning intent:

- `MAJOR`: incompatible public API or behavior changes.
- `MINOR`: backward-compatible functionality additions.
- `PATCH`: backward-compatible fixes.

During `0.x`, breaking changes may happen between minor versions. Breaking changes must be called out explicitly in release notes.

## MSRV (Minimum Supported Rust Version)

Current policy:

- Stable Rust is the required baseline (`rust-toolchain.toml`).
- Older compiler versions are not guaranteed unless explicitly documented in release notes.
- Any future fixed MSRV pin must be documented in this file and reflected in CI.

## Release Notes Contract

Each release should include:

- user-visible behavior changes
- breaking changes and migration notes
- security-impacting fixes
- benchmark/regression highlights when relevant
