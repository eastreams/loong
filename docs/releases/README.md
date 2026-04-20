# Release Directory

This directory is the tracked public release trail for Loong.

Top-level files here should stay focused on shipped release history, not on
release-governance support material.

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read public release history | the `vX.Y.Z*.md` notes and `*-announcement.md` files in this directory | those are the public release artifacts |
| compare versions through the broader public history | `../../CHANGELOG.md` and GitHub Releases | those stay canonical for release history outside the raw repo tree |
| prepare or review release-support material | [`support/README.md`](support/README.md) | maintainer workflow and templates live there |

## What Belongs At The Top Level

- tracked release notes such as `v0.1.0-alpha.2.md`
- shorter announcement-style artifacts such as `v0.1.0-alpha.2-announcement.md`

## What Lives Under `support/`

- release-governance workflow docs
- release document templates

## Boundary Rules

- Public readers should usually start with release notes, `CHANGELOG.md`, or
  GitHub Releases.
- Support files remain in the repository because scripts and governance checks
  depend on them.
- Support files should stay under `support/` rather than mixing with the public
  release trail.
- Local `.docs/` traces and debug logs are never part of the public docs path.
