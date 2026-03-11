# CLI Name Unification Design

**Scope**

Unify the canonical command name on `alpha-test` to `loongclaw` everywhere that
the repository exposes or documents a runnable CLI.

This includes:

- the compiled binary name
- clap help / command identity
- install scripts
- source-level operator guidance strings
- README and examples
- release workflow packaging inputs

This change explicitly does **not** keep a legacy compatibility alias.

**Problem Statement**

The repository already presents itself as `LoongClaw` at the project level, and
release archives are already named `loongclaw-*`. The mismatch is that the
actual executable and most command examples still use a daemon-suffixed command
name.

That split creates three operator problems:

- users have to remember a daemon-suffixed command that does not match the
  product name
- install, help, and follow-up guidance are inconsistent with release artifact
  naming
- future docs and automation will keep leaking mixed terminology unless the
  binary identity itself changes

**Chosen Design**

Make `loongclaw` the only canonical CLI name by changing the binary definition
and every user-facing reference that currently points to the daemon-suffixed
legacy command.

Implementation follows four layers:

1. rename the binary target in `crates/daemon/Cargo.toml`
2. rename the clap command identity in `crates/daemon/src/main.rs`
3. update all source strings that instruct operators which command to run
4. update scripts, docs, examples, and release workflow inputs so build,
   install, and published artifacts all speak the same name

**Behavior**

After this change:

- `cargo build -p loongclaw-daemon --bin loongclaw` is the intended build path
- help output identifies the CLI as `loongclaw`
- generated operator guidance says `loongclaw setup`, `loongclaw chat`, and
  `loongclaw doctor --fix`
- install scripts place `loongclaw` in the destination prefix
- release workflow packages the `loongclaw` executable inside
  `loongclaw-vX.Y.Z-*` archives

There is no fallback daemon-style binary name, alias, or compatibility wording
left behind.

**Risks And Mitigations**

- Existing scripts outside the repository that call the old daemon-suffixed
  command will break.
  This is an intentional compatibility break requested for the branch.
- CI / release workflow can silently drift if only docs are changed. Mitigation:
  update workflow variables and build invocations together with source.
- User guidance strings are easy to miss. Mitigation: add focused tests around
  the canonical command name and run a repository-wide search before
  completion.
