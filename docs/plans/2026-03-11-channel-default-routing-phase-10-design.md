# Channel Default Routing Phase 10 Design

**Scope**

Phase 10 makes the default-account routing decision observable and actionable.

Phase 9 already blocks ambiguous config integrity errors such as duplicate
normalized account ids and unknown `default_account` references. What remains
is a softer but still operationally important ambiguity: when LoongClaw falls
back to a default account implicitly, operators cannot currently tell whether
that default came from an explicit config choice, a compatibility mapping, or a
sorted fallback.

**Problem Statement**

Today `channels` and `doctor` can tell an operator which configured account is
currently treated as the default, but not why.

That missing "why" matters because these states are not equivalent:

- `default_account` explicitly points at one configured account
- one configured account is literally named `default`
- there is only one configured account, so fallback is harmless
- there are multiple configured accounts, and LoongClaw is silently picking the
  first sorted id
- there are no configured accounts, and the effective configured-account view is
  derived from runtime identity compatibility

Only one of those is truly risky for routing: the multi-account sorted fallback.
That is also the exact Telegram case OpenClaw warns about upstream.

**Reference Findings**

OpenClaw already models default-account provenance explicitly in Feishu:

- `explicit-default`
- `mapped-default`
- `fallback`

OpenClaw's Telegram account layer emits a warning when:

- there are multiple configured accounts
- no explicit `defaultAccount` is set
- no configured `default` account exists
- routing therefore falls back to the first sorted configured account id

That is the right standard for LoongClaw too. Not because LoongClaw must mimic
every upstream detail, but because it expresses the real operational risk.

**Chosen Design**

Add a first-class default-account selection source to LoongClaw's channel
config/runtime substrate.

For Telegram and Feishu/Lark, resolve:

- `explicit_default`
- `mapped_default`
- `fallback`
- `runtime_identity`

The default-account id remains what it is today; this phase adds provenance.

**Selection Semantics**

1. If `accounts` is non-empty and `default_account` matches a configured account
   after normalization:
   - source = `explicit_default`
2. Else if `accounts` is non-empty and one configured account id is `default`:
   - source = `mapped_default`
3. Else if `accounts` is non-empty:
   - source = `fallback`
   - selected id = first sorted configured account id
4. Else:
   - source = `runtime_identity`
   - selected id = single-account compatibility identity already used by
     Phase 9

**Operator Surface Changes**

`ChannelStatusSnapshot` should include:

- `default_account_source`
- `is_default_account`

`channels` text output should show both the current default marker and the
selection source.

`doctor` should warn only in the genuinely risky case:

- multi-account channel
- default snapshot source = `fallback`

That warning should tell the operator that omitting `--account` depends on a
sorted fallback and should recommend setting `default_account` explicitly.

**Why This Is The Right Next Step**

This phase tightens account routing without overreaching into runtime
supervision or Discord adapter work.

It gives LoongClaw:

- explicit default-routing provenance
- actionable operator warnings for risky fallback routing
- a cleaner substrate for future multi-account supervisors and Discord

After this phase, the next major missing layer is no longer default-account
semantics. It is the shared runtime orchestration substrate for long-lived
channel workers.
