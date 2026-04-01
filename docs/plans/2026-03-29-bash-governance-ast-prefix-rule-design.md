# Bash Exec AST Governance Design

**Status:** Approved-for-planning draft based on the design decisions confirmed in this thread.

## Relation to Issue #637 and PR #677

This design is the next implementation slice under issue `#637` (`Shell command governance: Bash execution and Bash rule parsing support`).

It builds on PR `#677`, which added the basic `bash.exec` runtime surface, discoverability, runtime visibility, and execution plumbing.

This design does **not** complete the whole issue. It narrows the next slice to:

- AST-backed minimal-command-unit splitting for `bash.exec`
- prefix-rule evaluation for governable Bash command units
- conservative fallback to `Default(Allow|Deny)` for unsupported or unresolved structures

It intentionally does **not** include:

- `shell.exec` governance changes
- `approval_required`
- `approve_once`
- `approve_always`
- any broader approval-flow integration

## Goal

Add a conservative, parser-backed governance layer for `bash.exec` that:

- uses Bash AST structure to split a command into minimal command units
- evaluates governable units with simple prefix rules
- falls back to the existing global default mode when the structure is unsupported or unresolved

The goal is to make `bash.exec` governable without pretending LoongClaw already has a full shell semantic engine.

## Why This Shape

Issue `#637` and its attached research point toward the same practical pattern seen in comparable tools:

- **Gemini CLI** demonstrates that parser-backed compound-command splitting is useful, especially for `&&`, `||`, `;`, and newline-separated command lists.
- **Codex** demonstrates that the user-facing rule language can stay simple (`prefix_rule(...)`) while shell parsing remains an internal conservative normalization layer rather than a public AST DSL.
- **OpenCode** demonstrates that a simple permission surface can still be improved internally by parser-assisted command extraction and conservative aggregation.

The research does **not** support making the first LoongClaw slice a public AST predicate language. The clearer path is:

1. keep the user-facing first slice simple
2. use Bash parsing internally
3. fall back conservatively when semantics are unclear

That is the design adopted here.

## Scope

Included in this slice:

- `bash.exec`-only governance
- `tree-sitter-bash` parsing for command analysis
- minimal-command-unit splitting for a conservative subset of Bash list forms
- prefix-rule evaluation for plain simple-command units
- compatibility with existing `shell_allow` / `shell_deny` configuration by translating them into single-token prefix rules
- reuse of `tools.shell_default_mode` as the global `Default(Allow|Deny)` mode for Bash governance
- structured analysis output sufficient for denial reasons and future audit expansion

Excluded from this slice:

- `shell.exec` changes
- approval semantics of any kind
- rule outcomes beyond `allow`, `deny`, and `Default`
- AST predicates, regex rules, wildcard rules, or richer rule languages
- fine-grained governance for pipelines, redirections, substitutions, env-prefix assignments, subshells, compound commands, functions, loops, or conditionals
- persistent shell state or session semantics
- multi-layer rule overlays or dynamic hot reload

## User-Facing Rule Surface

### Rules Directory

The default home-scoped rules directory should follow a Codex-like shape:

- `~/.loongclaw/rules/`

This design assumes LoongClaw loads Bash governance rules from `*.rules` files in that directory.

The directory choice is intentional:

- it is easy to discover
- it keeps shell-governance rules separate from unrelated runtime config
- it leaves room for multiple focused rule files instead of one monolithic config block

### Rule Language

The user-facing rule language should be Starlark-backed, but the exposed first-slice rule function is intentionally small:

```python
prefix_rule(
    pattern = ["cargo", "test"],
    decision = "allow",
)

prefix_rule(
    pattern = ["cargo", "publish"],
    decision = "deny",
)
```

The rule language in this slice is **not** an AST DSL. Users are not writing predicates over node kinds, redirections, substitutions, or shell structure.

### Compatibility With Existing `shell.exec` Config

This slice must remain compatible with the existing `shell.exec` command allow/deny configuration:

- `tools.shell_allow`
- `tools.shell_deny`

Compatibility works by translating each configured bare command name into a one-token prefix rule:

- `shell_allow = ["cargo"]` behaves like an allow prefix rule for `["cargo"]`
- `shell_deny = ["rm"]` behaves like a deny prefix rule for `["rm"]`

Those translated rules participate in the same `bash.exec` evaluator as `.loongclaw/rules/*.rules`.

This is intentionally prefix-based compatibility, not direct reuse of the old raw-string semantics.

If compatibility rules and `.loongclaw/rules/*.rules` both define the same prefix:

- deny still has higher precedence than allow regardless of source
- same-decision duplicates are harmless and may be normalized away during rule compilation

## Default Mode

This slice must reuse the existing global default mode:

- `tools.shell_default_mode`

It remains the source of truth for `Default(Allow|Deny)` when:

- no explicit prefix rule matches a governable unit
- a command unit contains unsupported structure
- parsing cannot produce a reliable governable decomposition

This avoids creating a second competing default-mode concept under `tools.bash`.

## Parsing Model

### Parser Choice

Use `tree-sitter-bash` as the Bash parser.

This is sufficient for the slice because it provides:

- a Rust-native integration path
- structured syntax nodes for Bash command forms
- explicit parse error / missing-node detection so LoongClaw can fail closed instead of pretending it understood malformed input

### Parse Failure Behavior

If parsing produces a tree with errors, missing nodes, or recovery artifacts that make the top-level structure unreliable, the evaluator must not continue with partial governance claims.

In that case:

- the whole `bash.exec` command falls back to `Default(Allow|Deny)`

This is an intentional conservative choice.

## Minimal Command Unit Model

### Supported Outer Splitting Forms

The first slice should only split on outer list structures that are conservative and easy to explain:

- `;`
- newline-separated lists
- `&&`
- `||`

This means LoongClaw can split examples such as:

- `cd foo && cargo test`
- `cargo fmt ; cargo test`
- `cargo test || cargo test -- --nocapture`

### Execution-Potential Rule

The splitting model must follow the same rule stated in issue `#637`:

> if a command unit may execute, it must be covered by either an explicit rule or the configured default policy

That means:

- both sides of `&&` are relevant
- both sides of `||` are relevant
- LoongClaw does not exclude the right-hand side of `||` merely because it might not run at runtime

### Governable Unit Definition

A minimal command unit is considered governable in this slice only when it is a plain simple command whose argv prefix can be extracted with confidence and without inventing semantics for unsupported shell structures.

Examples of governable units:

- `pwd`
- `cd foo`
- `cargo test`
- `cargo test -- --nocapture`

For these units, LoongClaw extracts a normalized argv sequence and runs prefix-rule matching on that argv.

That normalization is lexical rather than semantic:

- ordinary quoting and escaping are resolved into the argv token text before prefix comparison
- unsupported shell structures such as substitutions, redirections, and env-prefix assignments do not enter this normalization path at all because those units never become governable plain-command units in the first place

## Unsupported Structures

Even if the parser can recognize them syntactically, the following structures are **not** given fine-grained governance semantics in this slice:

- environment-variable prefix assignment
- redirection
- heredoc / herestring
- pipeline
- subshell
- command substitution
- process substitution
- function definitions
- loop forms
- conditional compound forms

Examples:

- `FOO=1 cargo test`
- `cargo test > out.txt`
- `cargo test | tee out.txt`
- `(cd foo && cargo test)`
- `echo $(git rev-parse HEAD)`

These structures do **not** get downgraded into a plain command for prefix matching.

For example:

- `FOO=1 cargo test` is **not** treated as equivalent to plain `cargo test`
- `cargo test > out.txt` is **not** treated as equivalent to plain `cargo test`
- `cargo test | tee out.txt` is **not** split into a governable `cargo test` unit plus a second pipeline artifact

Instead, the affected minimal command unit resolves to `Default`.

This is an explicit scope boundary, not a parser limitation claim.

## Decision Model

### Unit-Level Results

Each minimal command unit yields one of three internal results:

- `Allow`
- `Deny`
- `Default`

For governable plain simple-command units, evaluation order is:

1. explicit deny prefix rules
2. explicit allow prefix rules
3. `Default`

This preserves deny precedence.

For unsupported units:

- the result is `Default`

### Whole-Command Aggregation

The whole `bash.exec` command aggregates unit results as follows:

1. if any unit resolves to `Deny`, the whole command is `Deny`
2. otherwise, if every unit resolves to `Allow`, the whole command is `Allow`
3. otherwise, if at least one unit resolves to `Default`, the whole command resolves through `tools.shell_default_mode`

This means a mixed command such as:

- `git status && cargo test | tee out.txt`

is handled as:

- `git status` -> can be explicitly allowed or denied by prefix rule
- `cargo test | tee out.txt` -> `Default`

and the final result is **not** automatically upgraded to `Allow` just because one unit matched an allow rule.

## Evaluator Architecture

This slice should introduce a dedicated Bash governance evaluator that runs only for `bash.exec`.

The intended execution flow is:

1. `bash.exec` receives a command string
2. the evaluator parses the command with `tree-sitter-bash`
3. the evaluator splits the command into minimal command units where supported
4. the evaluator classifies each unit as governable or unsupported
5. the evaluator applies prefix rules plus `tools.shell_default_mode`
6. if the result is `Allow`, `bash.exec` continues to execution
7. if the result is `Deny`, `bash.exec` returns a policy denial before execution

This design deliberately avoids:

- changing `shell.exec`
- widening the current kernel policy contract
- pretending the existing approval flow is already the right primitive for this slice

## Rule Loading and Failure Behavior

### Rules Directory Discovery

The first slice should load Bash governance rules from:

- `.loongclaw/rules/*.rules`

using stable lexical ordering.

### Missing Rules Directory

If the directory is absent, LoongClaw should treat that as:

- no explicit rule files

and continue with:

- translated compatibility rules from `shell_allow` / `shell_deny`
- otherwise `Default`

### Broken Rule Files

If a configured rule file exists but cannot be read, parsed, or compiled, LoongClaw must fail closed for the affected governance load rather than silently reverting to an empty rule set.

The system must not use rule-load failure as a hidden path to capability widening.

## Denial and Analysis Output

Even without approval semantics, denial output must be explainable enough to distinguish:

- explicit deny rule matches
- default-deny because no allow rule matched
- default-deny because the unit used unsupported structure
- fallback due to parse unreliability

The evaluator should therefore produce structured analysis including:

- raw command string
- parse success / failure state
- minimal command unit list
- unit classifications
- unit-level decision sources
- final aggregated decision

This structured result is useful immediately for denial reporting and later for stronger audit or approval summaries if a later slice adds them.

## Testing Requirements

The first implementation plan for this design must prove at least the following:

- plain simple commands can match allow prefix rules
- plain simple commands can match deny prefix rules
- both sides of `&&` are evaluated
- both sides of `||` are evaluated as potentially executable units
- mixed governable / unsupported commands use unit-level `Default` plus whole-command aggregation
- parse-error input falls back to whole-command `Default`
- `shell_allow` compatibility entries behave as one-token allow prefix rules
- `shell_deny` compatibility entries behave as one-token deny prefix rules
- `tools.shell_default_mode = allow` and `deny` both affect unresolved Bash governance outcomes correctly
- `shell.exec` behavior remains unchanged

## Explicit Non-Goals

This design intentionally does not answer:

- how `approval_required` should fit into Bash governance
- how `approve_once` or `approve_always` should behave
- how to extend these rules to `shell.exec`
- how to expose AST predicates publicly
- how to add regex or wildcard rule matching
- how to precisely govern redirection, pipelines, substitutions, env-prefix assignments, or compound shell control structures
- how to add layered rule scopes, policy overlays, or hot reload

Those are follow-up topics, not implied commitments of this slice.
