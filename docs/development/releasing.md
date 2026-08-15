# Releasing `loac`

This repository publishes two crates:

- `loac-macros`, the procedural macros;
- `loac`, the actor runtime.

They use one version. Publish the macro crate first.

## Prepare a release

Work on a release commit that will land on `rewrite`.

1. Update `version` in both package manifests:
   `crates/actor-macros/Cargo.toml` and `crates/actor/Cargo.toml`.
2. Update the runtime's `loac-macros` requirement to that version.
   Keep its `path` entry for workspace development.
3. Update the examples link in `crates/actor/src/lib.rs`.
   It must use `loac-v<VERSION>`.
4. Run `cargo check --workspace` to refresh `Cargo.lock`.
5. Review the generated package metadata and commit the release.

The workspace version is unrelated. Do not change it for this release.

The macro crate's `actor-api` dev-dependency uses both `path` and `version`.
Workspace tests use the local runtime through `path`; `cargo package` strips
`path`, so published macro archives resolve the `version` from crates.io. Keep
that version at the last published `loac` release — currently `0.2.0` — and do
not advance it to the new actor version until that release is on crates.io.
This lets macro archive tests run before the new actor release.

Before pushing the release commit, run the local source check:

```bash
bash scripts/release-loac.sh check
bash scripts/release-loac.sh verify-package loac-macros
```

The second crate needs the macro version on crates.io first.

## Configure GitHub

The workflow is `.github/workflows/release-loac.yml`.

Create a GitHub environment named `release`. Add this environment secret:

```text
CARGO_REGISTRY_TOKEN=<crates.io API token>
```

The token needs permission to publish both crates. Keep it in the environment.
Do not put it in the repository or workflow file.

GitHub exposes manual dispatch only when the workflow exists on the default
branch. Keep the workflow there, normally after review merges it to `dev`.
The job itself checks out `rewrite`, so push the release commit there first.
That checkout must be clean. The workflow also needs permission to push tags.

## Run the release

Open **Actions**, select **Release loac**, and choose **Run workflow**.
Run it from the branch containing the workflow, normally `dev`.
The job then checks out `rewrite` and performs these steps:

1. Run formatting, tests, and documentation checks.
2. Package and test `loac-macros` from its archive.
3. Publish `loac-macros` when that exact archive is absent.
4. Wait until the macro archive is visible on crates.io.
5. Package and test `loac` from its archive.
6. Create and push the annotated immutable tag `loac-v<VERSION>`.
7. Publish `loac` when that exact archive is absent.

The script compares archive checksums with crates.io. An existing version with
different contents stops the workflow. Published versions are never replaced.

## Retry a failed run

Retry the same workflow after fixing a transient failure.

Already completed steps are idempotent:

- a matching published archive is skipped;
- a matching local or remote tag is reused;
- a different archive or tag target fails closed.

Do not change source files after the first crate is published. A new release
needs a new version and a new release commit.

## Verify the release

After the workflow succeeds, verify the tag and both registry entries:

```bash
VERSION=0.2.0
git fetch origin --tags
git show --stat "loac-v$VERSION"
cargo info "loac@$VERSION"
cargo info "loac-macros@$VERSION"
```

Then check the published docs and run a small consumer build. The examples
index in those docs must resolve through the immutable release tag.
