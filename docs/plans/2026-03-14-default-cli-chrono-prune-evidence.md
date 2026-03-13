## Summary

This note records the evidence behind pruning `chrono` from the default
`loongclaw` CLI path.

The change is intentionally narrow:

- replace the remaining default-binary `chrono` usage in provider retry policy
- replace the remaining default-binary `chrono` usage in onboarding backup
  timestamp formatting
- keep user-visible behavior intact

## Behavioral Scope

The retained behavior is:

- numeric `Retry-After` values continue to parse as seconds
- RFC 2822 HTTP-date `Retry-After` values continue to parse
- RFC 3339 `Retry-After` values are covered explicitly
- onboarding backup filenames retain the existing
  `YYYYMMDD-HHMMSS` shape

## Dependency Rationale

Before this cleanup, the default CLI path still pulled `chrono` through narrow
call sites in:

- `crates/app/src/provider/policy.rs`
- `crates/daemon/src/onboard_cli.rs`

On macOS, that dependency had previously introduced a direct CoreFoundation
link through the `chrono` -> `iana-time-zone` -> `core-foundation-sys` chain.

The intended outcome of this change is therefore:

- a smaller default release binary
- a smaller default dependency surface
- no claim of app-internal startup optimization unless benchmark data supports
  it

## Measurement Notes

The detailed startup and binary-size study was executed in a dedicated
performance analysis worktree rather than in this PR branch.

That dedicated study found:

- the stripped default binary shrank from `6,078,368` bytes to `6,009,968`
  bytes
- the direct dynamic dependency surface dropped the old CoreFoundation link
- alternating startup reruns did not reproduce a stable regression
- `main_entry_to_prompt_ms` stayed effectively unchanged, so any observed
  startup improvement is more defensibly described as pre-`main()` residual
  improvement rather than app-internal startup optimization

This PR should therefore be described conservatively:

- it is a dependency and footprint cleanup for the default CLI path
- it is consistent with earlier measured pre-`main()` improvement
- it should not be presented as proof of faster app-internal startup

## PR-Branch Validation

The code changes on this branch are validated with:

- targeted onboarding backup-path tests
- targeted provider retry-policy tests
- the full `loongclaw-app` test suite
- a fresh default release build for `loongclaw`
