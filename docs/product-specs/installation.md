# Installation

## User Story

As a new LoongClaw user, I want one documented install path that is easy to run
and honest about what it can do today, so that I can get to `onboard`, `ask`,
or `chat` without reverse-engineering release or source workflows.

## Acceptance Criteria

- [x] Product docs expose a bootstrap installer path for Linux/macOS and
      Windows.
- [x] The bootstrap installer prefers GitHub Release binaries, verifies their
      SHA256 checksums, installs the matching `loong` binary when a release
      exists for the requested version, and keeps `loongclaw` as a compatible
      entrypoint.
- [x] Linux x86_64 release artifacts distinguish GNU and musl variants, and the
      Bash installer auto-selects GNU only when the host satisfies the declared
      GNU glibc floor; otherwise it falls back to musl.
- [x] If the repository has not published a matching release yet, the installer
      fails with an explicit next action instead of constructing a misleading or
      broken download URL.
- [x] Product docs keep a source-install path for repository users and document
      the explicit `--source` fallback from a local checkout.
- [x] The install path can hand users directly into `loong onboard` after a
      successful install.
- [x] When the shell already exposes exactly one ready credential-backed web
      search provider, the installer prefers that provider before falling back
      to locale and route heuristics.
- [x] Linux users can explicitly override the libc variant when they need a
      specific GNU or musl artifact.

## Out of Scope

- Package-manager distribution (`brew`, `apt`, `winget`, etc.)
- Auto-update or self-update flows
- Signed package notarization and installer branding
