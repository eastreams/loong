# Installation

## User Story

As a new LoongClaw user, I want one documented install path that is easy to run
and honest about what it can do today, so that I can get to `onboard`, `ask`,
or `chat` without reverse-engineering release or source workflows.

## Acceptance Criteria

- [x] Product docs expose a bootstrap installer path for Linux/macOS and
      Windows.
- [x] The bootstrap installer prefers GitHub Release binaries, verifies their
      SHA256 checksums, and installs the matching `loongclaw` binary when a
      release exists for the requested version.
- [x] If the repository has not published a matching release yet, the installer
      fails with an explicit next action instead of constructing a misleading or
      broken download URL.
- [x] Product docs keep a source-install path for repository users and document
      the explicit `--source` fallback from a local checkout.
- [x] The install path can hand users directly into `loongclaw onboard` after a
      successful install.

## Out of Scope

- Package-manager distribution (`brew`, `apt`, `winget`, etc.)
- Auto-update or self-update flows
- Signed package notarization and installer branding
