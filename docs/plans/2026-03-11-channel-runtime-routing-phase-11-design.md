# Channel Runtime Routing Phase 11 Design

**Scope**

Phase 11 pushes default-account provenance from static operator surfaces into the
actual runtime entrypoints for Telegram and Feishu/Lark.

Phase 10 made default selection observable in `channels` and `doctor`, but the
runtime commands themselves still silently accept risky implicit routing when an
operator omits `--account` in a multi-account setup.

**Problem Statement**

Today LoongClaw can tell an operator after the fact that default routing came
from a fallback selection, but it does not use that provenance when launching:

- `telegram-serve`
- `feishu-send`
- `feishu-serve`

That leaves a real operational gap:

- the configuration layer knows a selected account came from fallback
- `doctor` can warn about it
- the runtime command still proceeds without an inline warning

This is exactly the moment where a routing surprise matters most.

**Reference Direction**

OpenClaw Telegram warns at runtime only in the risky case:

- multiple configured accounts
- no explicit `defaultAccount`
- no configured `accounts.default`
- operator omitted account selection

LoongClaw should now do the same thing with its own stronger provenance model.

**Chosen Design**

Add a reusable resolved-route view that captures:

- requested account id, if any
- selected configured account id
- configured account count
- default-account selection source

From that route view, derive one shared predicate:

- `uses_implicit_fallback_default`

This predicate becomes the runtime gate for inline warnings.

**Behavior**

For Telegram and Feishu/Lark:

1. Build a resolved route view after account resolution.
2. If the operator explicitly passed `--account`, do not warn.
3. If only one configured account exists, do not warn.
4. If the default source is `explicit_default`, `mapped_default`, or
   `runtime_identity`, do not warn.
5. If the operator omitted `--account` and the selected account comes from
   multi-account fallback routing:
   - emit a warning before launch/send
   - include the selected configured account id
   - recommend setting `<channel>.default_account` or passing `--account`

Also surface the route source inline in runtime banners so runtime behavior and
`channels` / `doctor` stay consistent.

**Why This Phase Matters**

This is the smallest meaningful slice of a shared channel supervisor substrate.

It does not attempt to build a full supervisor yet, but it does establish a
shared runtime-routing context that future long-lived channel workers and a
Discord adapter can reuse instead of re-implementing account-routing heuristics.
