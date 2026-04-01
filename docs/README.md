# Repository Docs Map

This directory is the repository-native documentation map for LoongClaw.

The public reader-facing docs surface lives under [`site/`](../site/README.md).
The files under `docs/` remain useful because they are the source-facing
references, specs, and maintainer support material that back the public docs
site and repository workflow.

Not every file under `docs/` belongs in the normal public reading path.

## Route By Audience

| If you are trying to... | Start here | Why |
| --- | --- | --- |
| read LoongClaw like a public docs site visitor | [`../site/index.mdx`](../site/index.mdx) | `site/` is the main reader-facing docs surface |
| understand runtime shape, boundaries, and crate responsibilities | [`design-docs/index.md`](design-docs/index.md) | source-facing architecture references live there |
| edit or review user-facing behavior contracts | [`product-specs/index.md`](product-specs/index.md) | product-spec markdown is the source contract behind the docs site |
| check roadmap, reliability, product, or security references from the repository | [`ROADMAP.md`](ROADMAP.md), [`RELIABILITY.md`](RELIABILITY.md), [`PRODUCT_SENSE.md`](PRODUCT_SENSE.md), [`SECURITY.md`](SECURITY.md) | these are the repository-native reference documents |
| understand contributor fit and repo-native references | [`../CONTRIBUTING.md`](../CONTRIBUTING.md) and [`references/README.md`](references/README.md) | contribution guidance and supporting references are split intentionally |
| follow release history as a public reader | [`../site/reference/releases.mdx`](../site/reference/releases.mdx) | the docs site keeps the public release path clearer than the raw repository directory |
| prepare or maintain release-governance docs | [`releases/support/README.md`](releases/support/README.md) | release support conventions are maintainer material |

## Directory Roles

| Path | Role |
| --- | --- |
| `design-docs/` | source-facing architecture references |
| `product-specs/` | source-facing user and operator requirements |
| `references/` | supporting contributor references and maintainer support docs |
| `releases/` | public release notes and announcements, with support material isolated under `releases/support/` |

## Boundary Rules

- `site/` is the main public docs surface.
- `docs/` stays source-facing and repository-native.
- maintainer support artifacts may remain in `docs/` when scripts, issue
  templates, or governance checks depend on them.
- those maintainer artifacts should not be promoted as normal reader-facing docs
  unless they become part of the public product or contributor contract.

## Do Not Put Here By Default

- new landing-page style onboarding or tutorial content that belongs under
  `site/`
- backlog-heavy plans, internal comparison notes, or working design bundles
- temporary authoring notes that do not serve a stable public or repository
  support contract
- duplicated mirrors of Mintlify navigation pages
