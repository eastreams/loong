# Code Standards

This document defines mandatory code standards for Loong source changes.
The English and Simplified Chinese versions of this document must describe the
same rule set.

## Scope

These rules apply to new and modified repository code unless a rule explicitly
narrows its scope.

Vendored third-party source is exempt only when it lives under an explicitly
vendored path.

This document does not replace `AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`,
`docs/design-docs/core-beliefs.md`, or
`docs/design-docs/layered-kernel-design.md`.

## Rule Terms

- `MUST` means the rule is required.
- `MUST NOT` means the pattern is forbidden.

## Rust Code Shape

### RUST-1: Public APIs Are Additive

Public APIs MUST remain additive unless the change links to a documented
breaking-change decision.

### RUST-2: Layer Boundaries Are Mandatory

Lower-layer crates MUST NOT contain domain-specific behavior unless the relevant
architecture document explicitly permits that dependency direction.

### RUST-3: New Dependencies Require A Reason

New workspace dependencies MUST include a PR note naming the dependency, the
owning crate, and the reason existing workspace dependencies or the standard
library are insufficient.

### RUST-4: Forbidden Rust Patterns Stay Forbidden

Rust code MUST NOT use `unwrap`, `expect`, `panic`, `todo`, `unimplemented`,
unsafe code, stdout debug prints, or stderr debug prints.

## Function Size

### SIZE-1: Functions Are Limited To 50 Lines By Default

Every Rust function and method MUST be at most 50 physical source lines unless
the function itself is annotated with `#[allow(clippy::too_many_lines)]`.

Line counting starts at the `fn` signature line and ends at the matching closing
brace. Attributes and doc comments before the signature do not count. Blank
lines inside the function do count.

### SIZE-2: Oversized Function Exceptions Require Reviewer Rationale

Any function or method over 50 counted lines MUST carry
`#[allow(clippy::too_many_lines)]` on that function or method.

The author MUST explain the exception reason to reviewers in the PR, review
thread, or adjacent source comment.

### SIZE-3: Oversized Functions Must Be Extracted

When a function would exceed 50 counted lines, validation, transformation,
execution, formatting, or setup work MUST be extracted into smaller named
functions unless the function follows `SIZE-2`.

## Test Organization

### TEST-1: Private Behavior Tests Stay With The Module

Tests for private helpers or private module behavior MUST live in the same Rust
source file as the implementation.

### TEST-2: Public Behavior Tests Use Integration Surfaces

Tests for public CLI, runtime, protocol, or cross-crate behavior MUST live under
the owning crate's `tests/` directory or the workspace `tests/` directory.

### TEST-3: Unit-Test-Only Helpers Use test_utils.rs

Functions used only by embedded or unit test modules, and not used by production
code, MUST live in a `test_utils.rs` file.

The `test_utils.rs` module declaration MUST be guarded by `#[cfg(test)]`.
Production code MUST NOT import `test_utils.rs`.

### TEST-4: Integration-Test Support Uses test_support.rs

Helpers that must be compiled into a crate for integration tests, including mock
providers, fake transports, harness builders, and integration fixtures, MUST
live in `test_support.rs`.

The `test_support.rs` module and its public exports MUST be guarded by both
`#[cfg(test)]` and the `dev-test-support` feature. The `dev-test-support`
feature MUST NOT be included in the crate's default feature set and MUST NOT be
enabled by release build commands.

### TEST-5: Test Module Names Are Restricted

Rust test modules MUST be named `tests` or start with `tests_`.

### TEST-6: Test Modules Require Test Guards

Rust test modules MUST be guarded by `#[cfg(test)]`.

### TEST-7: Embedded Test Modules Are Last

Embedded Rust test modules MUST appear at the end of the source file.

### TEST-8: Production Code Cannot Follow Embedded Tests

Production code MUST NOT appear after an embedded Rust test module.

### TEST-9: Tests Must Not Use Real User State

Tests MUST NOT read from or write to the developer's real home directory. Tests
that need Loong state MUST set `LOONG_HOME` to an isolated temporary directory or
use `./scripts/cargo-local-toolchain.sh test`, which provides an isolated default
test home.

### TEST-10: Live Network Tests Are Isolated

Tests MUST NOT perform live network calls unless they are explicitly marked as
ignored or gated behind a feature that is disabled by default.

## Test Naming

### NAME-1: Test Functions Use Behavior Names

Rust test function names MUST describe the behavior being protected in
`snake_case`.

### NAME-2: Vague Test Names Are Forbidden

Test function names MUST NOT be `test_basic`, `test_success`, `test_error`,
`test_failure`, `test_regression`, or the same names with numeric suffixes.

### NAME-3: Issue IDs Cannot Replace Behavior Names

Bug IDs, issue IDs, and incident IDs MUST NOT be the only behavior description in
a test name. Put IDs in comments when they are needed.

## Fixtures And Snapshots

### DATA-1: Fixtures Live Next To Their Owning Tests

Fixtures MUST live under a `fixtures/` directory adjacent to the test file that
owns them. Shared fixtures MUST live under a shared `fixtures/` directory with a
README that names the owning test suites.

### DATA-2: Fixture Names Describe The Scenario

Fixture filenames MUST include the domain or behavior under test. Fixture
filenames MUST NOT use only `sample`, `test`, `temp`, `tmp`, `data`, or numeric
names.

## Error Handling And Observability

### ERR-1: Security-Critical Outcomes Are Observable

Policy denials, capability failures, token lifecycle changes, and plane
invocations MUST emit the audit evidence required by the kernel and reliability
documentation.

### ERR-2: Security-Critical Results Must Be Handled

Results from policy, capability, audit, and security-scan operations MUST be
handled explicitly. `let _ = ...` is forbidden for those operations.

### ERR-3: Generic Error Strings Cannot Hide Governed Failures

New or modified governed runtime code MUST NOT collapse policy denials,
capability failures, or audit-write failures into generic error strings.
