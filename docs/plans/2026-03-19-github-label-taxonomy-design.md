# GitHub Label Taxonomy Refresh Design

**Problem**

LoongClaw's current GitHub label setup has three problems:

1. Pull requests receive an automatic `rust` label even though Rust is the default implementation
   language for almost the whole repository, so the label adds little routing value.
2. The managed subsystem labels use `area:` prefixes and identical colors, which makes the queue
   harder to scan and weaker than the more direct label naming style used by OpenClaw.
3. The taxonomy is duplicated across `.github/labeler.yml`, `.github/workflows/labeler.yml`,
   issue-form dropdowns, and `docs/references/github-collaboration.md`, so every rename or color
   adjustment risks drift.

**Goal**

Replace the current prefixed `area:*` and `domain:*` labels with clearer unprefixed labels, retire
automatic `rust` tagging, improve color differentiation, and make one checked-in taxonomy source
drive the GitHub workflow, path labeling config, issue forms, and contributor docs.

**Non-Goals**

- Do not redesign the broader issue/PR process beyond label naming and routing.
- Do not add a large label matrix or heavy workflow framework.
- Do not infer roadmap-domain labels from file paths; those remain manual planning labels.
- Do not add a second bot or external service for label management.

**Root Cause**

The visible problems are color collisions and noisy names, but the underlying cause is taxonomy
duplication. The repository currently hardcodes the same subsystem list in several files. That made
the initial rollout quick, but it now makes even small naming changes brittle and encourages
partial edits. The right fix is not another one-off rename in each file; the right fix is a single
taxonomy definition that the repo can regenerate and verify.

**Reference**

OpenClaw's label set is a good reference for naming style:

- labels read like product surfaces (`agents`, `gateway`, `cli`, `docs`)
- prefixes are kept only for real families such as `channel:` and `extensions:`
- colors differentiate label families instead of flattening everything into one blue bucket

LoongClaw should adopt the same naming discipline while keeping its own surfaces and roadmap
structure.

**Approaches Considered**

1. Hand-edit the existing workflow, labeler config, issue forms, and docs.
   Rejected because it preserves the current duplication and guarantees future drift.

2. Move all label behavior into a larger custom GitHub Action.
   Rejected because it increases moving parts for a problem that is mostly static data.

3. Add a checked-in taxonomy manifest plus a small generator/checker script.
   Recommended because it keeps the solution simple, local, reviewable, and easy to verify in CI.

**Chosen Design**

Introduce a machine-readable taxonomy file under `.github/` that defines:

- managed surface labels
- managed roadmap-domain labels
- shared general managed labels
- issue-form surface options and their human-readable prompt text
- path globs for automatic pull-request labeling

Then add a lightweight script that renders or validates:

- `.github/labeler.yml`
- `.github/workflows/labeler.yml`
- `.github/ISSUE_TEMPLATE/*.yml`
- `docs/references/github-collaboration.md`

The taxonomy will use two explicit groups:

1. **Surface labels** for routing concrete repo areas such as `kernel`, `protocol`, `daemon`,
   `providers`, `tools`, `browser`, `channels`, `memory`, `conversation`, `config`, `acp`,
   `migration`, `docs`, and `ci`.
2. **Domain labels** for higher-level roadmap slices such as `core-runtime`,
   `agent-experience`, `tools-plugins`, `channels-protocol`, and `platform-dx`.

The names will drop the `area:` and `domain:` prefixes entirely. Descriptions and documentation
will explain the grouping instead of encoding the group in the label name itself.

**Color Strategy**

Surface labels will no longer share one color. They will use a small but readable palette grouped by
function:

- kernel/contracts/protocol/spec: blue family
- daemon/providers/tools/browser/channels: teal and indigo family
- memory/conversation/config/acp/migration: violet and magenta family
- docs/ci: cyan and orange family

Domain labels will keep stronger distinct colors because they are broader planning lenses and
should be visually separable from surface labels at a glance.

**Automation Behavior**

- Pull requests still receive automatic surface labels from file paths.
- Pull requests still receive one `size:*` label.
- Pull requests no longer receive the automatic `rust` label.
- Issue forms continue to sync one selected surface label.
- Domain labels remain managed and documented, but are applied manually instead of path-derived.

**Verification Plan**

Add a governance regression test that:

- fails if `rust` remains in the generated managed or path-labeled sets
- fails if generated files still contain `area:` or `domain:` prefixed managed labels
- fails if generated artifacts drift from the taxonomy manifest

This gives the repository an explicit regression gate for future label changes instead of relying on
human memory.
