## Summary

- Problem:
- Why it matters:
- What changed:
- What did not change (scope boundary):

## Linked Issues

- Closes #
- Related #

## Change Type

- [ ] Bug fix
- [ ] Feature
- [ ] Refactor
- [ ] Documentation
- [ ] Security hardening
- [ ] CI / workflow / release

## Touched Areas

- [ ] Kernel / policy / approvals
- [ ] Contracts / protocol / spec
- [ ] Daemon / CLI / install
- [ ] Providers / routing
- [ ] Tools
- [ ] Browser automation
- [ ] Channels / integrations
- [ ] ACP / conversation / session runtime
- [ ] Memory / context assembly
- [ ] Config / migration / onboarding
- [ ] Docs / contributor workflow
- [ ] CI / release / workflows

## Risk Track

- [ ] Track A (routine / low-risk)
- [ ] Track B (higher-risk / policy-impacting)

If Track B, fill these in:

- Risk notes:
- Rollout / guardrails:
- Rollback path:

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `cargo test --workspace --all-features --locked`
- [ ] Relevant architecture / dep-graph / docs checks for touched areas
- [ ] Additional scenario, benchmark, or manual checks when behavior changed
- [ ] If this changes config/env fallback, limits, or defaults: include before/after behavior and regression coverage for explicit path, fallback path, and boundary values
- [ ] If tests mutate process-global env: document how state is restored or serialized

Commands and evidence:

```text
Paste the exact commands you ran and summarize the result.
```

## User-visible / Operator-visible Changes

- None, or describe the exact change:

## Failure Recovery

- Fast rollback or disable path:
- Observable failure symptoms reviewers should watch for:

## Reviewer Focus

- Point reviewers at the files, edge cases, or risk seams that deserve the most attention.
