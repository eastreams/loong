# First-Run Assistant Journey Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Productize the first-run LoongClaw assistant journey by adding `loongclaw ask`, shipping a built-in SSRF-safe `web.fetch` tool, and aligning product-facing docs/specs with the real MVP path.

**Architecture:** Reuse the current CLI conversation bootstrap and turn coordinator for `ask`; extend the existing tool config/catalog/runtime execution plane for `web.fetch`; update product docs in the same PR so the shipped behavior and documented journey stay aligned.

**Tech Stack:** Rust, clap, serde, reqwest blocking client, existing LoongClaw config/runtime/tool abstractions, Markdown docs.

---

### Task 1: Add the `ask` CLI surface

**Files:**
- Modify: `crates/daemon/src/main.rs`
- Modify: `crates/daemon/src/tests/mod.rs`

**Step 1: Write the failing test**

Add CLI/help tests that prove:

- `ask` exists as a subcommand
- help text explains it as a one-shot CLI prompt
- `--message` is required

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-daemon cli_ask`

Expected: FAIL because the `ask` subcommand/help does not exist yet.

**Step 3: Write minimal implementation**

- Add `Commands::Ask` in `crates/daemon/src/main.rs`
- wire subcommand dispatch to a new `run_ask_cli(...)`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-daemon cli_ask`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/daemon/src/main.rs crates/daemon/src/tests/mod.rs
git commit -m "feat(cli): add ask command surface"
```

### Task 2: Reuse chat bootstrap for one-shot ask

**Files:**
- Modify: `crates/app/src/chat.rs`
- Test: `crates/app/src/chat.rs`
- Modify: `crates/daemon/src/main.rs`

**Step 1: Write the failing test**

Add app-level unit tests that prove a new one-shot path:

- resolves the session hint like `chat`
- prints a single assistant reply and exits
- rejects disabled CLI config the same way as `chat`

Prefer testing extracted helper behavior instead of end-to-end stdin plumbing.

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app cli_ask`

Expected: FAIL because the one-shot helper/path does not exist.

**Step 3: Write minimal implementation**

- extract shared CLI bootstrap from `run_cli_chat(...)`
- add `run_cli_ask(config_path, session_hint, message, options)`
- call `ConversationTurnCoordinator::handle_turn_with_address_and_acp_options(...)`
- print `loongclaw> {assistant_text}` once and exit

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app cli_ask`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/chat.rs crates/daemon/src/main.rs
git commit -m "feat(cli): implement one-shot ask flow"
```

### Task 3: Add `tools.web` config and runtime policy

**Files:**
- Modify: `crates/app/src/config/tools_memory.rs`
- Modify: `crates/app/src/runtime_env.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`

**Step 1: Write the failing test**

Add tests that prove:

- `ToolConfig` includes a default `web` section
- TOML parsing accepts `tools.web.enabled`, domain allow/block rules, limits, and local-host override
- runtime environment exports the matching env vars/runtime policy

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tools_memory:: runtime_env:: tool_runtime_config::`

Expected: FAIL because the `web` policy fields are not defined yet.

**Step 3: Write minimal implementation**

- add `WebToolConfig` to `ToolConfig`
- add `WebFetchRuntimePolicy` to `ToolRuntimeConfig`
- export env/runtime state from `initialize_runtime_environment(...)`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools_memory:: runtime_env:: tool_runtime_config::`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/config/tools_memory.rs crates/app/src/runtime_env.rs crates/app/src/tools/runtime_config.rs
git commit -m "feat(tools): add web fetch runtime policy"
```

### Task 4: Expose `web.fetch` in the tool catalog

**Files:**
- Modify: `crates/app/src/tools/catalog.rs`
- Modify: `crates/app/src/tools/mod.rs`

**Step 1: Write the failing test**

Add tests that prove:

- `web.fetch` appears in the runtime tool view when enabled
- provider tool definitions include the `web_fetch` schema
- capability snapshot / tool registry mention `web.fetch`
- alias canonicalization accepts `web_fetch`

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app tools::`

Expected: FAIL because `web.fetch` is not registered or advertised yet.

**Step 3: Write minimal implementation**

- add a tool descriptor and provider schema in `catalog.rs`
- gate runtime view exposure on `config.tools.web.enabled`
- extend canonical-name handling and tool registry expectations in `tools/mod.rs`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app tools::`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/tools/catalog.rs crates/app/src/tools/mod.rs
git commit -m "feat(tools): advertise web fetch capability"
```

### Task 5: Implement the `web.fetch` executor

**Files:**
- Create: `crates/app/src/tools/web_fetch.rs`
- Modify: `crates/app/src/tools/mod.rs`
- Test: `crates/app/src/tools/web_fetch.rs`

**Step 1: Write the failing test**

Add tests that prove:

- disabled runtime blocks `web.fetch`
- private/loopback/local/reserved targets are denied by default
- explicit local-test override can allow localhost for integration tests
- redirects are rejected or revalidated safely
- response size/time limits are enforced
- readable content is returned with unsafe/script/style noise removed

**Step 2: Run test to verify it fails**

Run: `cargo test -p loongclaw-app web_fetch`

Expected: FAIL because the executor does not exist yet.

**Step 3: Write minimal implementation**

- validate URL scheme/host
- resolve DNS/IP targets and block unsafe ranges by default
- execute request with bounded redirect handling and timeout
- cap response bytes
- extract readable text/metadata
- route dispatch in `execute_tool_core_with_config(...)`

**Step 4: Run test to verify it passes**

Run: `cargo test -p loongclaw-app web_fetch`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/app/src/tools/web_fetch.rs crates/app/src/tools/mod.rs
git commit -m "feat(tools): implement ssrf-safe web fetch"
```

### Task 6: Refresh product specs and first-run docs

**Files:**
- Modify: `README.md`
- Modify: `docs/PRODUCT_SENSE.md`
- Modify: `docs/product-specs/index.md`
- Create: `docs/product-specs/onboarding.md`
- Create: `docs/product-specs/doctor.md`
- Create: `docs/product-specs/channel-setup.md`
- Create: `docs/product-specs/webchat.md`
- Create: `docs/product-specs/one-shot-ask.md`

**Step 1: Write the failing test**

Use document assertions by checking:

- product spec index links the new specs
- product sense command table includes `ask` and removes stale `setup`
- README quick start uses `onboard` then `ask/chat`
- README visible tools section mentions `web.fetch`

**Step 2: Run test to verify it fails**

Run: `rg -n "setup|ask|web.fetch|onboarding|doctor|channel setup|WebChat" README.md docs/PRODUCT_SENSE.md docs/product-specs`

Expected: current docs are incomplete or stale.

**Step 3: Write minimal implementation**

- add the missing product specs
- update existing docs to reflect shipped behavior

**Step 4: Run test to verify it passes**

Run: `rg -n "setup|ask|web.fetch|onboarding|doctor|channel setup|WebChat" README.md docs/PRODUCT_SENSE.md docs/product-specs`

Expected: docs/specs align with the new first-run path.

**Step 5: Commit**

```bash
git add README.md docs/PRODUCT_SENSE.md docs/product-specs docs/plans/2026-03-15-first-run-assistant-journey-design.md docs/plans/2026-03-15-first-run-assistant-journey-implementation-plan.md
git commit -m "docs(product): define first-run assistant journey"
```

### Task 7: Verify, publish, and open PR

**Files:**
- Modify if needed: `.github/PULL_REQUEST_TEMPLATE.md` only for reference, not content changes

**Step 1: Run focused verification**

Run:

```bash
cargo test -p loongclaw-daemon cli_ask
cargo test -p loongclaw-app cli_ask
cargo test -p loongclaw-app web_fetch
```

Expected: PASS

**Step 2: Run broader verification**

Run:

```bash
cargo fmt --all --check
cargo test -p loongclaw-app
cargo test -p loongclaw-daemon
```

Expected: PASS

**Step 3: Inspect clean delivery state**

Run:

```bash
git status --short
git diff --cached --name-only
git diff --cached
```

Expected: only task-scoped changes remain.

**Step 4: Push and open PR**

- push branch to `fork-chumyin`
- open PR against `loongclaw-ai/loongclaw:alpha-test`
- use repository PR template
- include `Closes #168`
- mention that `web.fetch` advances the product track in `#41`

**Step 5: Final confirmation**

Report:

- verification commands executed
- tests that passed
- PR URL
