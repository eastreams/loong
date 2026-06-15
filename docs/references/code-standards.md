# Code Standards

This document defines repository-native code standards for Loong maintainers,
contributors, and agent-assisted work.

Use it for rules that are more concrete than the project principles in
`docs/design-docs/core-beliefs.md`, but broader than one crate or one feature.
When a rule can be enforced mechanically, prefer a script, lint, or CI gate over
manual review.

## Scope

This document covers:

- Rust code shape and maintainability expectations
- function, module, and file size guidance
- test placement and test file organization
- fixture, snapshot, and generated-output handling
- error-handling and observability conventions
- review checks for code-standard drift
- current and planned enforcement hooks

## Rust Code Shape

Record project-wide Rust style rules here when they go beyond rustfmt and
Clippy defaults.

- Prefer explicit, narrow data flow over hidden global state.
- Keep public APIs additive unless a documented breaking-change decision exists.
- Keep domain-specific behavior out of lower-layer crates unless the relevant
  architecture document explicitly allows it.
- Prefer local helper functions only when they make call sites clearer or remove
  meaningful duplication.
- Avoid introducing new dependencies for small utilities that can be expressed
  clearly with the standard library or existing workspace dependencies.

## Function And Module Size

Use this section for concrete size budgets and refactoring triggers.

- Functions should stay small enough that their control flow can be reviewed in
  one pass.
- Long functions need a clear reason, such as table-driven parsing, structured
  command wiring, or test setup that is easier to read inline.
- Split functions when separate validation, transformation, execution, or
  rendering steps can be named and tested independently.
- Keep modules focused on one responsibility. If a file accumulates unrelated
  helper families, prefer moving helpers next to the feature or crate surface
  that owns them.

Proposed mechanical budgets should be added here before they are enforced in
scripts or CI.

| Surface | Target | Enforcement |
| --- | --- | --- |
| Function length | Decide threshold before enforcing | Manual review for now |
| File length | Decide threshold before enforcing | Manual review for now |
| Test module length | Decide threshold before enforcing | Manual review for now |

## Test File Organization

Use tests to document behavior, not only to raise coverage numbers.

- Unit tests should live next to the implementation when they exercise private
  helpers or narrow module behavior.
- Integration tests should live under the crate or workspace test surface that
  matches the public behavior being exercised.
- Test files should group by behavior or runtime surface, not by incidental bug
  report history.
- Prefer deterministic inputs and outputs. Tests should not depend on a real
  home directory, live network service, wall-clock timing, or shared mutable
  host state unless the test explicitly exists to validate that integration.
- Keep large setup helpers and fixtures named by domain so future agents and
  reviewers can find the reason they exist.

## Test Naming

Test names should make the behavior and expectation clear.

- Name tests after the rule or scenario they protect.
- Avoid vague names such as `test_basic`, `test_error`, or `test_success` when
  the behavior has a more precise name.
- Use regression references in comments or PR text when useful, but keep the
  test name focused on the behavior that must remain true.

## Fixtures, Snapshots, And Generated Output

Fixtures and snapshots are part of the reviewed contract.

- Store fixtures near the tests that own them unless multiple crates or suites
  intentionally share them.
- Keep fixture data minimal and purpose-built.
- Do not update snapshots or generated outputs without reviewing the semantic
  change they represent.
- Generated files should identify the generator and should not be manually
  edited unless their header explicitly allows it.

## Error Handling And Observability

Error paths should be explicit enough for users, operators, and agents to debug
without reading unrelated source files first.

- Return structured errors or typed diagnostics when the surrounding crate
  already has that pattern.
- Do not hide policy denials, capability failures, or audit-write failures behind
  generic strings.
- Security-critical behavior should produce audit evidence as required by the
  kernel and reliability documentation.
- Avoid `unwrap`, `expect`, `panic`, `todo`, and `unimplemented` in production
  code. Workspace Clippy settings already reject these patterns.

## Review Checklist

Use this checklist when reviewing code-standard-sensitive changes:

- Does the change preserve the crate dependency direction?
- Are functions and modules still focused enough to review locally?
- Are new tests placed where future maintainers will expect them?
- Do test names describe durable behavior rather than implementation details?
- Are fixtures, snapshots, and generated files intentionally changed?
- Are error paths explicit and observable?
- Can any repeated review comment be turned into a script, lint, or CI check?

## Enforcement

Current mechanical enforcement includes:

- `cargo fmt` through `./scripts/cargo-local-toolchain.sh fmt --all -- --check`
- strict Clippy through `./scripts/cargo-local-toolchain.sh clippy --workspace --all-targets --all-features -- -D warnings`
- workspace tests through `./scripts/cargo-local-toolchain.sh test --workspace`
- all-feature tests through `./scripts/cargo-local-toolchain.sh test --workspace --all-features`
- dependency and architecture checks through `scripts/check_dep_graph.sh` and
  `scripts/check_architecture_boundaries.sh`

Future size, test-layout, or fixture checks should be documented here before
being added to local verification or CI.
