# Mintlify Docs Source

This directory is the public Mintlify docs source for LoongClaw.

Current scope:

- keep the docs site focused on public, reader-facing documentation
- avoid mirroring internal plans, private design backlog, or repository-only analysis
- keep the Mintlify site English-only for now
- make `site/` the main reader-facing docs surface instead of growing more repository-only landing-page text

Language policy:

- the main repository supports Simplified Chinese only for `README.zh-CN.md`
- public docs-site source under `site/` stays English-only until a dedicated docs i18n workflow is intentionally introduced
- if docs i18n is added later, it should be scoped to the Mintlify site rather than expanding repository-wide markdown translation

Future i18n introduction:

- start with Mintlify locale routing and translated navigation under `site/`
- keep English as the canonical authoring base unless a maintained per-locale workflow is added
- translate reader-facing setup guides and playbooks before deeper repository reference material
- do not expand repo-native translations beyond `README.zh-CN.md` by default

Local preview:

- Mintlify CLI currently expects a supported Node LTS release. Do not use Node 25+ for local preview.
- The `mintlify dev` preview path can take noticeably longer than expected on first warmup. If the CLI
  appears stuck in `preparing local preview`, verify your Node version first before assuming the
  docs content is broken.

```bash
node -v
cd site
npx mintlify@4.2.464 dev
```

If your shell is pinned to a newer global Node, use a temporary verified Node 20 toolchain instead of changing the whole shell first:

```bash
cd site
NODE20_BIN="$(npm exec --yes --package=node@20.20.2 -- node -p 'process.execPath')"
PATH="$(dirname "$NODE20_BIN"):$PATH" npm exec --yes --package=mintlify@4.2.464 -- mintlify dev
```

One-time local preview without a global install still uses the same `mintlify` entrypoint.

Validation before publishing:

```bash
cd site
NODE20_BIN="$(npm exec --yes --package=node@20.20.2 -- node -p 'process.execPath')"
PATH="$(dirname "$NODE20_BIN"):$PATH" npm exec --yes --package=mintlify@4.2.464 -- mintlify broken-links
PATH="$(dirname "$NODE20_BIN"):$PATH" npm exec --yes --package=mintlify@4.2.464 -- mintlify export --output /tmp/loongclaw-docs-export.zip
```

`broken-links` is the quickest structural check. `export` is the stronger proof that the docs can
be compiled into a full static site even when the local live preview is slow to warm up.
Pinning both the Node and Mintlify versions avoids two common false negatives:
registry tag drift on the Mintlify package, and local shells that still resolve to unsupported Node 25+.
The PATH override is necessary because the Mintlify CLI re-spawns a plain `node` process from the current shell path.

CI parity:

- GitHub Actions now runs the same Mintlify `broken-links` and `export` checks on pushes and pull requests in the `docs-site` job.
- local validation should stay command-for-command compatible with CI so docs failures are reproducible before review.

Mintlify repository setup:

- repository root: this repository
- monorepo path: `/site`
- config file: `site/docs.json`
- when configuring Mintlify monorepo deployment, the docs path should stay `/site` without a trailing slash
- no custom docs domain or checked-in `CNAME` is configured in this repository today; do not invent a public docs URL in README copy until the Mintlify deployment is actually connected

Mintlify connection checklist:

1. Open Mintlify dashboard `Git Settings` and install the Mintlify GitHub App for this repository.
2. Grant repository access only to the LoongClaw repo that actually hosts the docs source.
3. Choose the publishing branch that should trigger production docs updates.
4. Enable monorepo mode and set the docs path to `/site` with no trailing slash.
5. Verify Mintlify resolves `site/docs.json` from that path before treating the site as connected.
6. Confirm one pull request preview and one branch deployment before announcing a public docs URL.
7. If the GitHub organization uses extra network restrictions or IP allowlists, follow Mintlify's current GitHub connection guidance in the dashboard docs before debugging sync failures.

The current scaffold keeps a compact information architecture:

- `Overview`
- `Get Started`
- `Use LoongClaw`
- `Build On LoongClaw`
- `Reference`

Supporting public markdown remains under `docs/` as repository reference material and source-facing documentation. It should support the site and contributor workflow rather than grow into a second landing surface.

Repository-docs boundary:

- `README.md` and `README.zh-CN.md` are landing pages, not full manuals
- `site/` is the main reader-facing docs surface
- `docs/` remains the public repository markdown source and reference archive
- new reader-facing guidance should prefer the site instead of making the README or repo indexes longer
- backlog-heavy design packages and comparative analysis should stay out of the public docs path

Deployment notes:

- Mintlify can deploy automatically from the connected GitHub repository through the GitHub App
- the repository-side definition of "deployment ready" is: CI can validate the site, and Mintlify dashboard connection is configured against this repo
- this repository should be connected as a monorepo with `site/` as the docs source
- if deployment is added for docs i18n in the future, it should live at the Mintlify site layer rather than by expanding repository-wide markdown translation
