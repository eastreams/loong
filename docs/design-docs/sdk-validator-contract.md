# SDK Validator Contract

## Purpose

This document defines what Loong validates for public capability artifacts.

It exists to answer one practical question:

- what should fail deterministically, what should warn, and what belongs to
  doctor, install, or runtime policy instead of package validation?

## Core Distinction

Validator behavior must stay separate from:

- doctor
- install or activation
- runtime policy

Validation answers whether an artifact is structurally and semantically valid
for its family.

Doctor answers whether the current operator environment is ready.

Install or activation performs governed mutation.

Runtime policy answers whether the current session or product mode may use or
acquire the capability right now.

## Result Contract

Every validator surface should converge on:

- `valid`
- `valid_with_warnings`
- `invalid`

## Diagnostic Meaning

Validator diagnostics should be able to communicate:

- artifact family
- artifact identity when known
- severity
- category
- short summary
- subject path or field when available
- remediation hint when available

Recommended severities:

- `error`
- `warning`
- `note`

Recommended categories:

- `discovery`
- `structure`
- `identity`
- `metadata`
- `ownership`
- `runtime_lane`
- `promotion`
- `provenance`

## Validation Phases

Loong should validate public capability artifacts in this sequence:

1. discovery and root resolution
2. structural safety
3. identity normalization
4. metadata and setup semantics
5. runtime-lane and ownership compatibility
6. promotion boundedness and evidence semantics when applicable

## Families

### Managed skills

Managed-skill validation should enforce:

- exactly one installable root
- root contains `SKILL.md`
- archive and path safety
- symlink rejection where managed installs depend on local boundaries
- a normalizable `skill_id`

### Governed plugin packages

Plugin validation should enforce:

- manifest-first package discovery
- deterministic package-vs-source conflict handling
- required identity fields
- setup metadata structure
- ownership intent
- controlled runtime lanes

### Promotion artifacts

Promotion validation should enforce:

- explicit schema version, surface, and purpose
- supported target taxonomy
- bounded scope
- required capabilities
- provenance and readiness semantics

## Stability

Validator meaning should be treated as Additive moving toward Stable.

Loong should stabilize the meaning of:

- errors vs warnings
- major diagnostic categories
- the separation between validation, doctor, install, and policy

before trying to freeze exact renderer shape.
