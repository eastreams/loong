# Agent Role Resolution Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce an explicit internal `AgentRole` for delegate children, persist it in the
constrained-subagent execution contract, surface it in runtime prompts and session inspection, and
keep the external delegate request schema and current enforcement semantics unchanged.

**Architecture:** Add `AgentRole` in the conversation subagent contract layer, resolve it once at
delegate spawn time, and carry it through `ConstrainedSubagentExecution`,
`ConstrainedSubagentContractView`, `SessionContext`, prompt assembly, and `session_status`
observability. Keep `DelegateBuiltinProfile` as the public preset and keep
`ConstrainedSubagentProfile` focused on orchestration posture only.

**Tech Stack:** Rust, `serde`, existing conversation/session runtime code in `crates/app`,
`rusqlite`-backed session inspection tests, cargo fmt/clippy/test workspace gates.

---

### Task 1: Add failing role-resolution contract tests

**Files:**
- Modify: `crates/app/src/conversation/subagent.rs`
- Modify: `crates/app/src/conversation/mod.rs`

- [ ] **Step 1: Add the new enum and resolver API signatures to the test expectations**

Use these signatures as the target API for the tests in `subagent.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Default,
    Explorer,
    Worker,
    Verifier,
}

pub fn resolve_agent_role(
    profile: Option<DelegateBuiltinProfile>,
    depth: usize,
    max_depth: usize,
) -> AgentRole {
    let _ = (depth, max_depth);
    match profile {
        None => AgentRole::Default,
        Some(DelegateBuiltinProfile::Research) => AgentRole::Explorer,
        Some(DelegateBuiltinProfile::Plan) => AgentRole::Worker,
        Some(DelegateBuiltinProfile::Verify) => AgentRole::Verifier,
    }
}
```

- [ ] **Step 2: Add a failing resolver test matrix**

Add tests in `crates/app/src/conversation/subagent.rs` that assert:

```rust
#[test]
fn resolve_agent_role_maps_builtin_profiles_and_keeps_role_stable_at_leaf_depth() {
    assert_eq!(resolve_agent_role(None, 0, 2), AgentRole::Default);
    assert_eq!(
        resolve_agent_role(Some(DelegateBuiltinProfile::Research), 1, 2),
        AgentRole::Explorer
    );
    assert_eq!(
        resolve_agent_role(Some(DelegateBuiltinProfile::Plan), 1, 2),
        AgentRole::Worker
    );
    assert_eq!(
        resolve_agent_role(Some(DelegateBuiltinProfile::Verify), 1, 2),
        AgentRole::Verifier
    );
    assert_eq!(
        resolve_agent_role(Some(DelegateBuiltinProfile::Plan), 2, 2),
        AgentRole::Worker
    );
}
```

- [ ] **Step 3: Add a failing execution round-trip test for `agent_role`**

Extend the existing event-payload round-trip test shape so the execution envelope includes the new
field:

```rust
let execution = ConstrainedSubagentExecution {
    mode: ConstrainedSubagentMode::Async,
    isolation: ConstrainedSubagentIsolation::Shared,
    depth: 1,
    max_depth: 2,
    active_children: 0,
    max_active_children: 3,
    timeout_seconds: 60,
    allow_shell_in_child: false,
    child_tool_allowlist: vec!["file.read".to_owned()],
    workspace_root: None,
    runtime_narrowing: ToolRuntimeNarrowing::default(),
    kernel_bound: false,
    identity: None,
    profile: Some(ConstrainedSubagentProfile::for_child_depth(1, 2)),
    agent_role: Some(AgentRole::Explorer),
};

assert_eq!(
    ConstrainedSubagentExecution::from_event_payload(&payload)
        .expect("execution")
        .agent_role,
    Some(AgentRole::Explorer)
);
```

- [ ] **Step 4: Run the focused subagent tests and verify they fail**

Run:

```bash
cargo test -p loongclaw-app resolve_agent_role --features memory-sqlite
cargo test -p loongclaw-app constrained_subagent_execution_round_trips_event_payload --features memory-sqlite
```

Expected: FAIL because `AgentRole` and the new `agent_role` field do not exist yet.

### Task 2: Add failing prompt and session-status observability tests

**Files:**
- Modify: `crates/app/src/conversation/tests.rs`
- Modify: `crates/app/src/tools/session.rs`

- [ ] **Step 1: Replace the prompt expectation with role-based guidance**

Update the existing prompt test in `crates/app/src/conversation/tests.rs` so it expects a role
marker and role language instead of preset language:

```rust
let role_marker = "[delegate_child_role]";
let contract_marker = "[delegate_child_runtime_contract]";

assert!(
    merged.contains("You are running with the `worker` agent role."),
    "expected worker role guidance, got: {merged}"
);
assert!(
    merged.contains("- Turn findings into an execution plan."),
    "expected worker guidance bullet, got: {merged}"
);
assert!(
    merged.find(role_marker).expect("role marker")
        < merged.find(contract_marker).expect("contract marker"),
    "expected delegate role guidance before runtime contract, got: {merged}"
);
```

- [ ] **Step 2: Add a failing session-status assertion for `agent_role`**

Extend `session_status_includes_delegate_lifecycle_for_queued_child` in
`crates/app/src/tools/session.rs` so it asserts both the persisted execution and surfaced contract
include the resolved role:

```rust
assert_eq!(
    outcome.payload["delegate_lifecycle"]["execution"]["agent_role"],
    "explorer"
);
assert_eq!(
    outcome.payload["subagent_contract"]["agent_role"],
    "explorer"
);
```

- [ ] **Step 3: Add a failing fallback-contract test for legacy/partial history**

Add a focused test near the session-status fallback helpers that builds a child session with only
legacy depth information plus `profile: "verify"` and asserts the surfaced contract still reports
`agent_role == "verifier"`.

Use this target assertion:

```rust
assert_eq!(contract["agent_role"], "verifier");
assert_eq!(contract["profile"]["role"], "leaf");
```

- [ ] **Step 4: Run the focused prompt and session-status tests and verify they fail**

Run:

```bash
cargo test -p loongclaw-app default_runtime_build_context_includes_delegate_role_guidance_before_runtime_contract --features memory-sqlite
cargo test -p loongclaw-app session_status_includes_delegate_lifecycle_for_queued_child --features memory-sqlite
```

Expected: FAIL because the runtime still emits role guidance and the execution/contract JSON do
not expose `agent_role`.

### Task 3: Implement `AgentRole` in the constrained-subagent contract layer

**Files:**
- Modify: `crates/app/src/conversation/subagent.rs`
- Modify: `crates/app/src/conversation/mod.rs`
- Modify: `crates/app/src/conversation/workspace_isolation.rs`
- Modify: `crates/app/src/tools/runtime_config.rs`

- [ ] **Step 1: Add the enum, resolver, and contract fields**

Implement the enum and add an optional field on both contract structs:

```rust
pub struct ConstrainedSubagentContractView {
    pub mode: Option<ConstrainedSubagentMode>,
    pub identity: Option<ConstrainedSubagentIdentity>,
    pub profile: Option<ConstrainedSubagentProfile>,
    pub agent_role: Option<AgentRole>,
    // existing fields unchanged...
}

pub struct ConstrainedSubagentExecution {
    pub mode: ConstrainedSubagentMode,
    pub isolation: ConstrainedSubagentIsolation,
    pub depth: usize,
    pub max_depth: usize,
    pub active_children: usize,
    pub max_active_children: usize,
    pub timeout_seconds: u64,
    pub allow_shell_in_child: bool,
    pub child_tool_allowlist: Vec<String>,
    pub workspace_root: Option<PathBuf>,
    pub runtime_narrowing: ToolRuntimeNarrowing,
    pub kernel_bound: bool,
    pub identity: Option<ConstrainedSubagentIdentity>,
    pub profile: Option<ConstrainedSubagentProfile>,
    pub agent_role: Option<AgentRole>,
}
```

- [ ] **Step 2: Add role-aware helpers on execution and contract view**

Add the minimal helpers needed by the rest of the runtime:

```rust
impl ConstrainedSubagentContractView {
    pub fn with_agent_role(mut self, agent_role: AgentRole) -> Self {
        self.agent_role = Some(agent_role);
        self
    }

    pub fn resolved_agent_role(&self) -> Option<AgentRole> {
        self.agent_role
    }
}

impl ConstrainedSubagentExecution {
    pub fn resolved_agent_role(&self) -> AgentRole {
        self.agent_role.unwrap_or(AgentRole::Default)
    }

    pub fn with_resolved_agent_role(mut self, agent_role: AgentRole) -> Self {
        if self.agent_role.is_none() {
            self.agent_role = Some(agent_role);
        }
        self
    }
}
```

- [ ] **Step 3: Include `agent_role` when building contract views**

Update `ConstrainedSubagentContractView::from_execution(...)` so it carries the role:

```rust
pub fn from_execution(execution: &ConstrainedSubagentExecution) -> Self {
    Self {
        mode: Some(execution.mode),
        identity: execution.identity.clone(),
        profile: Some(execution.resolved_profile()),
        agent_role: execution.agent_role,
        depth_budget: Some(ConstrainedSubagentBudgetSnapshot {
            current: execution.depth,
            max: execution.max_depth,
        }),
        // existing fields unchanged...
    }
}
```

- [ ] **Step 4: Export `AgentRole` and fix direct struct literals**

Export the new type from `crates/app/src/conversation/mod.rs` and update the direct
`ConstrainedSubagentExecution` literals in:

```text
crates/app/src/conversation/workspace_isolation.rs
crates/app/src/conversation/tests.rs
crates/app/src/tools/runtime_config.rs
```

Use explicit values like:

```rust
agent_role: Some(AgentRole::Worker),
```

or:

```rust
agent_role: None,
```

where the test is intentionally exercising legacy fallback behavior.

- [ ] **Step 5: Re-run the focused subagent tests and verify they pass**

Run:

```bash
cargo test -p loongclaw-app resolve_agent_role --features memory-sqlite
cargo test -p loongclaw-app constrained_subagent_execution_round_trips_event_payload --features memory-sqlite
```

Expected: PASS.

### Task 4: Wire resolved roles into delegate spawn and session context

**Files:**
- Modify: `crates/app/src/conversation/turn_coordinator.rs`
- Modify: `crates/app/src/conversation/runtime.rs`

- [ ] **Step 1: Resolve the role at delegate spawn time**

Update `constrained_subagent_execution_for_delegate(...)` in
`crates/app/src/conversation/turn_coordinator.rs` so it stores the resolved role next to the
existing depth-derived profile:

```rust
let subagent_profile = ConstrainedSubagentProfile::for_child_depth(next_child_depth, max_depth);
let agent_role =
    crate::conversation::resolve_agent_role(delegate_policy.profile, next_child_depth, max_depth);

ConstrainedSubagentExecution {
    mode,
    isolation: delegate_policy.isolation,
    depth: next_child_depth,
    max_depth,
    active_children,
    max_active_children,
    timeout_seconds: delegate_policy.timeout_seconds,
    allow_shell_in_child: delegate_policy.allow_shell_in_child,
    child_tool_allowlist: delegate_policy.child_tool_allowlist.clone(),
    workspace_root,
    runtime_narrowing: delegate_policy.runtime_narrowing.clone(),
    kernel_bound: binding.is_kernel_bound(),
    identity: subagent_identity,
    profile: Some(subagent_profile),
    agent_role: Some(agent_role),
}
```

- [ ] **Step 2: Add role accessors on `SessionContext`**

Extend `SessionContext` in `crates/app/src/conversation/runtime.rs` with an optional role field and
resolution helpers:

```rust
pub struct SessionContext {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    pub profile: Option<DelegateBuiltinProfile>,
    pub agent_role: Option<AgentRole>,
    // existing fields...
}

pub fn with_agent_role(mut self, agent_role: AgentRole) -> Self {
    self.agent_role = Some(agent_role);
    self
}

pub fn resolved_agent_role(&self) -> Option<AgentRole> {
    self.subagent_execution
        .as_ref()
        .and_then(|execution| execution.agent_role)
        .or_else(|| {
            self.subagent_contract
                .as_ref()
                .and_then(ConstrainedSubagentContractView::resolved_agent_role)
        })
        .or_else(|| self.profile.map(|profile| resolve_agent_role(Some(profile), 0, 0)))
        .or_else(|| self.parent_session_id.is_none().then_some(AgentRole::Default))
}
```

- [ ] **Step 3: Backfill the role when loading persisted child sessions**

When building `SessionContext` from a persisted child snapshot, set the role from the execution if
present and fall back to the stored delegate profile for legacy events:

```rust
if let Some(profile) = snapshot.delegate_profile {
    session_context = session_context.with_profile(profile);
    if session_context.resolved_agent_role().is_none() {
        session_context = session_context.with_agent_role(resolve_agent_role(Some(profile), 0, 0));
    }
}
```

If `with_subagent_execution(...)` already carries `agent_role`, let that value win.

- [ ] **Step 4: Run the focused delegate spawn/runtime tests**

Run:

```bash
cargo test -p loongclaw-app handle_turn_with_runtime_delegate_child_can_reenter_when_max_depth_allows --features memory-sqlite
cargo test -p loongclaw-app trait_default_session_context_preserves_delegate_execution_contract --features memory-sqlite
```

Expected: PASS, with the execution contract now carrying `agent_role`.

### Task 5: Replace profile prompt guidance with role prompt guidance

**Files:**
- Modify: `crates/app/src/conversation/runtime.rs`
- Modify: `crates/app/src/conversation/tests.rs`

- [ ] **Step 1: Rename the prompt helper to role semantics**

Replace `delegate_child_profile_prompt_summary(...)` with a role-based helper that reads the
resolved role from `SessionContext`:

```rust
fn delegate_child_role_prompt_summary(session_context: &SessionContext) -> Option<String> {
    let _parent_session_id = session_context.parent_session_id.as_ref()?;
    let role = session_context.resolved_agent_role()?;
    let summary = match role {
        AgentRole::Default => return None,
        AgentRole::Explorer => concat!(
            "[delegate_child_role]\n",
            "You are running with the `explorer` agent role.\n",
            "- Gather evidence before conclusions.\n",
            "- Prefer reading files, web sources, and browser extraction over proposing edits.\n",
            "- Return concise findings, concrete references, and unresolved risks."
        ),
        AgentRole::Worker => concat!(
            "[delegate_child_role]\n",
            "You are running with the `worker` agent role.\n",
            "- Turn findings into an execution plan.\n",
            "- Prefer ordered steps, explicit assumptions, and acceptance criteria.\n",
            "- Do not claim implementation is complete when you only have a proposal."
        ),
        AgentRole::Verifier => concat!(
            "[delegate_child_role]\n",
            "You are running with the `verifier` agent role.\n",
            "- Try to falsify success claims before accepting them.\n",
            "- Prefer concrete checks, observed failures, and residual risk notes.\n",
            "- Report a clear verdict with evidence."
        ),
    };
    Some(summary.to_owned())
}
```

- [ ] **Step 2: Keep the fragment ordering unchanged**

Update the prompt assembly call site so role guidance still lands ahead of the runtime contract:

```rust
let delegate_role_contract = include_system_prompt
    .then(|| delegate_child_role_prompt_summary(session_context))
    .flatten();

append_runtime_prompt_fragment(
    &mut assembled,
    "delegate-child-role",
    delegate_role_contract,
);
```

Do not move the runtime contract fragment or the runtime-self-continuity fragment.

- [ ] **Step 3: Update the focused prompt test to green**

Rename the test to `default_runtime_build_context_includes_delegate_role_guidance_before_runtime_contract`
and run:

```bash
cargo test -p loongclaw-app default_runtime_build_context_includes_delegate_role_guidance_before_runtime_contract --features memory-sqlite
```

Expected: PASS with role guidance and the role marker ordered ahead of the runtime-contract marker.

### Task 6: Surface `agent_role` in session inspection and legacy fallback paths

**Files:**
- Modify: `crates/app/src/tools/session.rs`
- Modify: `crates/app/src/conversation/subagent.rs`

- [ ] **Step 1: Keep lifecycle execution JSON role-aware**

Because `session_delegate_lifecycle_json(...)` already serializes
`ConstrainedSubagentExecution::with_resolved_profile`, make sure the execution now serializes the
new `agent_role` field:

```rust
"execution": lifecycle
    .execution
    .map(ConstrainedSubagentExecution::with_resolved_profile),
```

No new wrapper is needed if the struct serializes `agent_role` directly.

- [ ] **Step 2: Add a role fallback when only profile metadata is available**

In the fallback path that builds a contract from lineage/profile state, attach a derived role when
the stored execution contract does not already have one:

```rust
let contract = ConstrainedSubagentContractView::from_profile(
    ConstrainedSubagentProfile::for_child_depth(depth, tool_config.delegate.max_depth),
)
.with_agent_role(resolve_agent_role(
    match lifecycle_profile {
        Some("research") => Some(DelegateBuiltinProfile::Research),
        Some("plan") => Some(DelegateBuiltinProfile::Plan),
        Some("verify") => Some(DelegateBuiltinProfile::Verify),
        _ => None,
    },
    depth,
    tool_config.delegate.max_depth,
));
```

If the lifecycle already produced a contract with `agent_role`, keep that value and do not
overwrite it.

- [ ] **Step 3: Extend the queued-child status test and the fallback test to green**

Run:

```bash
cargo test -p loongclaw-app session_status_includes_delegate_lifecycle_for_queued_child --features memory-sqlite
cargo test -p loongclaw-app session_delegate_lifecycle_prefers_execution_mode_when_history_is_partial --features memory-sqlite
```

Expected: PASS, with `agent_role` visible in the execution envelope and surfaced contract.

### Task 7: Run focused regression coverage and full verification

**Files:**
- Modify: none unless verification exposes a necessary fix

- [ ] **Step 1: Run the focused role slice**

Run:

```bash
cargo test -p loongclaw-app resolve_agent_role --features memory-sqlite
cargo test -p loongclaw-app constrained_subagent_execution_round_trips_event_payload --features memory-sqlite
cargo test -p loongclaw-app default_runtime_build_context_includes_delegate_role_guidance_before_runtime_contract --features memory-sqlite
cargo test -p loongclaw-app session_status_includes_delegate_lifecycle_for_queued_child --features memory-sqlite
cargo test -p loongclaw-app handle_turn_with_runtime_delegate_child_can_reenter_when_max_depth_allows --features memory-sqlite
```

- [ ] **Step 2: Run broader app coverage that touches the same seam**

Run:

```bash
cargo test -p loongclaw-app conversation:: --features memory-sqlite
cargo test -p loongclaw-app tools::session --features memory-sqlite
```

- [ ] **Step 3: Run repository verification gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --all-features
```

Expected: all green. Any unrelated baseline failure must be called out explicitly before claiming
this slice is ready.

### Task 8: Prepare the implementation commit

**Files:**
- Modify: implementation files from Tasks 1-6 plus the existing design and plan docs

- [ ] **Step 1: Inspect the final diff**

Run:

```bash
git status --short
git diff -- crates/app/src/conversation/subagent.rs crates/app/src/conversation/mod.rs crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs crates/app/src/conversation/workspace_isolation.rs crates/app/src/tools/session.rs crates/app/src/tools/runtime_config.rs docs/plans/2026-04-05-agent-role-resolution-slice-design.md docs/plans/2026-04-05-agent-role-resolution-slice-implementation-plan.md
```

Expected: only the agent-role slice and the corresponding plan/design docs are present.

- [ ] **Step 2: Commit with a scoped message**

Run:

```bash
git add crates/app/src/conversation/subagent.rs crates/app/src/conversation/mod.rs crates/app/src/conversation/runtime.rs crates/app/src/conversation/turn_coordinator.rs crates/app/src/conversation/tests.rs crates/app/src/conversation/workspace_isolation.rs crates/app/src/tools/session.rs crates/app/src/tools/runtime_config.rs docs/plans/2026-04-05-agent-role-resolution-slice-design.md docs/plans/2026-04-05-agent-role-resolution-slice-implementation-plan.md
git commit -m "feat(conversation): add explicit delegate agent roles"
```

- [ ] **Step 3: Capture the verification commands in the PR or handoff summary**

Record the exact commands and outcomes from Task 7 in the delivery summary so review can confirm
the role slice stayed additive and green.
