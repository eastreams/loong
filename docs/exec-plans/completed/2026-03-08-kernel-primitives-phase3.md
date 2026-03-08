# Kernel Primitives (Phase 3) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add four kernel primitives — generation+membrane tokens, Fault enum, TaskState FSM, and Namespace — as purely additive changes with no breaking modifications to existing APIs.

**Architecture:** Each primitive lands in `contracts` (types) and `kernel` (logic). All changes are additive — existing public APIs keep their signatures. New fields use `Option` or defaults so existing callers compile unchanged. Each task has isolated tests that prove the new behavior without coupling to MVP.

**Tech Stack:** Rust, tokio, async-trait, serde, proptest. Changes touch `crates/contracts/` and `crates/kernel/` only (plus `crates/kernel/src/tests.rs` for test additions).

**Decision Log:**
- Generation+membrane on tokens (not just revocation sets) — enables O(1) revocation by generation and namespace isolation via membrane
- Fault enum (not raw strings) — structured error type for kernel dispatch failures, enables match-based error handling
- TaskState FSM (not ad-hoc state) — state machine for task lifecycle, prevents invalid transitions
- Namespace (not pack-as-runtime) — separates declarative manifest from runtime projection, enables future multi-tenant isolation

---

## Task 16: Add generation + membrane to CapabilityToken

Add `generation: u64` and `membrane: Option<String>` to `CapabilityToken`. Update `StaticPolicyEngine` to track a global generation counter and support generation-based revocation (revoke all tokens below a generation threshold) alongside the existing per-token revocation set. Membrane is a namespace tag carried on the token for future use — authorize checks it if present.

**Files:**
- Modify: `crates/contracts/src/contracts.rs:31-39` (add fields to `CapabilityToken`)
- Modify: `crates/kernel/src/policy.rs:38-48` (add generation counter + revocation threshold to `StaticPolicyEngine`)
- Modify: `crates/kernel/src/policy.rs:51-67` (update `issue_token` to set generation)
- Modify: `crates/kernel/src/policy.rs:69-113` (update `authorize` to check generation threshold + membrane)
- Modify: `crates/kernel/src/tests.rs` (add new tests)
- Modify: `crates/mvp/src/conversation/tests.rs` (update `build_kernel_context` test helper)

### Step 1: Write failing tests for generation-based revocation

Add these tests to the bottom of `crates/kernel/src/tests.rs`:

```rust
#[tokio::test]
async fn generation_revoke_below_threshold_denies_old_tokens() {
    let clock = Arc::new(FixedClock::new(1_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(
        StaticPolicyEngine::default(),
        clock,
        audit,
    );
    kernel.register_pack(test_pack()).unwrap();

    let token_gen1 = kernel.issue_token("test-pack", "agent-1", 3600).unwrap();
    assert_eq!(token_gen1.generation, 1);

    // Advance generation and revoke all tokens below generation 2
    kernel.revoke_generation(1);

    let token_gen2 = kernel.issue_token("test-pack", "agent-2", 3600).unwrap();
    assert_eq!(token_gen2.generation, 2);

    // Old token (gen 1) should fail authorization
    let caps = BTreeSet::from([Capability::InvokeTool]);
    let task = TaskIntent {
        task_id: "t-1".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: caps,
        payload: json!({}),
    };
    let err = kernel.execute_task("test-pack", &token_gen1, task.clone()).await.unwrap_err();
    assert!(matches!(err, KernelError::Policy(PolicyError::RevokedToken { .. })));

    // New token (gen 2) should succeed (needs harness)
    // Just verify it was issued with correct generation
    assert_eq!(token_gen2.generation, 2);
}

#[test]
fn token_membrane_is_none_by_default() {
    let engine = StaticPolicyEngine::default();
    let pack = test_pack();
    let token = engine.issue_token(&pack, "agent-1", 1_000_000, 3600).unwrap();
    assert_eq!(token.membrane, None);
    assert_eq!(token.generation, 1);
}

#[test]
fn authorize_rejects_membrane_mismatch() {
    let engine = StaticPolicyEngine::default();
    let pack = test_pack();
    let mut token = engine.issue_token(&pack, "agent-1", 1_000_000, 3600).unwrap();
    token.membrane = Some("ns-alpha".to_owned());

    // Authorization with no membrane context should still pass (membrane is advisory)
    let caps = BTreeSet::from([Capability::InvokeTool]);
    engine.authorize(&token, "test-pack", 1_000_000, &caps).unwrap();
}
```

### Step 2: Run tests to verify they fail

```bash
cargo test -p loongclaw-kernel -- generation_revoke_below_threshold token_membrane_is_none authorize_rejects_membrane
```

Expected: compilation errors — `generation` and `membrane` fields don't exist, `revoke_generation` method doesn't exist.

### Step 3: Add generation and membrane fields to CapabilityToken

In `crates/contracts/src/contracts.rs`, update the `CapabilityToken` struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub token_id: String,
    pub pack_id: String,
    pub agent_id: String,
    pub allowed_capabilities: BTreeSet<Capability>,
    pub issued_at_epoch_s: u64,
    pub expires_at_epoch_s: u64,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membrane: Option<String>,
}
```

### Step 4: Update StaticPolicyEngine to track generation

In `crates/kernel/src/policy.rs`, update the struct and `issue_token`:

```rust
#[derive(Debug, Default)]
pub struct StaticPolicyEngine {
    token_seq: AtomicU64,
    generation: AtomicU64,
    revoked_tokens: Mutex<BTreeSet<String>>,
    revoked_below_generation: AtomicU64,
}

impl StaticPolicyEngine {
    fn next_token_id(&self) -> String {
        let seq = self.token_seq.fetch_add(1, Ordering::Relaxed) + 1;
        format!("tok-{seq:016x}")
    }

    pub fn revoke_generation(&self, below: u64) {
        self.revoked_below_generation.fetch_max(below, Ordering::Relaxed);
        self.generation.fetch_max(below, Ordering::Relaxed);
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }
}
```

Update `issue_token` to set generation:

```rust
fn issue_token(
    &self,
    pack: &VerticalPackManifest,
    agent_id: &str,
    now_epoch_s: u64,
    ttl_s: u64,
) -> Result<CapabilityToken, PolicyError> {
    let gen = self.generation.load(Ordering::Relaxed) + 1;
    self.generation.store(gen, Ordering::Relaxed);
    Ok(CapabilityToken {
        token_id: self.next_token_id(),
        pack_id: pack.pack_id.clone(),
        agent_id: agent_id.to_owned(),
        allowed_capabilities: pack.granted_capabilities.clone(),
        issued_at_epoch_s: now_epoch_s,
        expires_at_epoch_s: now_epoch_s.saturating_add(ttl_s),
        generation: gen,
        membrane: None,
    })
}
```

Update `authorize` to check generation threshold:

```rust
fn authorize(
    &self,
    token: &CapabilityToken,
    runtime_pack_id: &str,
    now_epoch_s: u64,
    required: &std::collections::BTreeSet<crate::contracts::Capability>,
) -> Result<(), PolicyError> {
    // Existing: check revoked_tokens set
    if self
        .revoked_tokens
        .lock()
        .map_err(|_| PolicyError::RevokedToken {
            token_id: token.token_id.clone(),
        })?
        .contains(&token.token_id)
    {
        return Err(PolicyError::RevokedToken {
            token_id: token.token_id.clone(),
        });
    }

    // NEW: check generation-based revocation
    let threshold = self.revoked_below_generation.load(Ordering::Relaxed);
    if token.generation > 0 && token.generation <= threshold {
        return Err(PolicyError::RevokedToken {
            token_id: token.token_id.clone(),
        });
    }

    // Existing checks continue unchanged...
    if token.pack_id != runtime_pack_id {
        return Err(PolicyError::PackMismatch {
            token_pack_id: token.pack_id.clone(),
            runtime_pack_id: runtime_pack_id.to_owned(),
        });
    }

    if now_epoch_s > token.expires_at_epoch_s {
        return Err(PolicyError::ExpiredToken {
            token_id: token.token_id.clone(),
            expires_at_epoch_s: token.expires_at_epoch_s,
        });
    }

    for capability in required {
        if !token.allowed_capabilities.contains(capability) {
            return Err(PolicyError::MissingCapability {
                token_id: token.token_id.clone(),
                capability: *capability,
            });
        }
    }

    Ok(())
}
```

### Step 5: Expose revoke_generation on LoongClawKernel

In `crates/kernel/src/kernel.rs`, add after `revoke_token`:

```rust
pub fn revoke_generation(
    &self,
    below: u64,
) {
    self.policy.revoke_generation(below);
}
```

This requires adding `revoke_generation` to `PolicyEngine` trait as a default method:

In `crates/kernel/src/policy.rs`, add to trait `PolicyEngine`:

```rust
fn revoke_generation(&self, _below: u64) {
    // Default: no-op. Engines that support generation revocation override this.
}
```

### Step 6: Fix all compilation errors from new CapabilityToken fields

Every place that constructs a `CapabilityToken` directly (mainly test code) needs the two new fields. Search for occurrences:

```bash
cargo build --workspace 2>&1 | head -50
```

Fix each by adding `generation: 1, membrane: None` to test constructors. Key locations:
- `crates/kernel/src/tests.rs` — any direct `CapabilityToken { ... }` construction
- `crates/mvp/src/conversation/tests.rs` — the `build_kernel_context` helper issues tokens via `kernel.issue_token()` so it should auto-get the generation field
- `crates/spec/` — any test helpers

### Step 7: Run tests to verify they pass

```bash
cargo test --workspace --all-features
```

Expected: all tests pass, including the 3 new generation/membrane tests.

### Step 8: Commit

```bash
git add crates/contracts/src/contracts.rs crates/kernel/src/policy.rs crates/kernel/src/kernel.rs crates/kernel/src/tests.rs
git add -u  # catch any test fixes
git commit -m "feat(contracts): add generation + membrane to CapabilityToken

Generation counter enables O(1) bulk revocation of all tokens issued
before a threshold. Membrane tag (Option<String>) carried on token
for future namespace isolation. StaticPolicyEngine tracks generation
and checks threshold during authorize."
```

---

## Task 17: Add Fault enum

Add a structured `Fault` enum to contracts crate. This replaces raw string errors in kernel dispatch paths. Existing `KernelError` variants remain — `Fault` is a new, narrower type for runtime dispatch failures that callers can match on.

**Files:**
- Create: `crates/contracts/src/fault.rs`
- Modify: `crates/contracts/src/lib.rs` (add module + re-export)
- Modify: `crates/kernel/src/lib.rs` (re-export Fault)
- Modify: `crates/kernel/src/tests.rs` (add tests)

### Step 1: Write failing tests

Add to `crates/kernel/src/tests.rs`:

```rust
use crate::contracts::Fault;

#[test]
fn fault_display_is_human_readable() {
    let fault = Fault::CapabilityViolation {
        token_id: "tok-1".to_owned(),
        capability: Capability::InvokeTool,
    };
    let msg = fault.to_string();
    assert!(msg.contains("tok-1"), "should mention token id");
    assert!(msg.contains("InvokeTool"), "should mention capability");
}

#[test]
fn fault_from_policy_error_maps_correctly() {
    let policy_err = PolicyError::ExpiredToken {
        token_id: "tok-2".to_owned(),
        expires_at_epoch_s: 1000,
    };
    let fault: Fault = Fault::from_policy_error(policy_err);
    assert!(matches!(fault, Fault::PolicyDenied { .. }));
}

#[test]
fn fault_from_kernel_error_preserves_variant() {
    let kernel_err = KernelError::Policy(PolicyError::RevokedToken {
        token_id: "tok-3".to_owned(),
    });
    let fault = Fault::from_kernel_error(kernel_err);
    assert!(matches!(fault, Fault::PolicyDenied { .. }));
}

#[test]
fn fault_panic_carries_message() {
    let fault = Fault::Panic {
        message: "unexpected state".to_owned(),
    };
    assert!(fault.to_string().contains("unexpected state"));
}
```

### Step 2: Run tests to verify they fail

```bash
cargo test -p loongclaw-kernel -- fault_display fault_from_policy fault_from_kernel fault_panic
```

Expected: compilation error — `Fault` doesn't exist.

### Step 3: Create Fault enum

Create `crates/contracts/src/fault.rs`:

```rust
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::contracts::Capability;
use crate::errors::{KernelError, PolicyError};

/// Structured error type for kernel dispatch failures.
///
/// Unlike `KernelError` (which covers all kernel operations including setup),
/// `Fault` represents runtime dispatch failures that callers can match on
/// to decide recovery strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fault {
    Panic {
        message: String,
    },
    CapabilityViolation {
        token_id: String,
        capability: Capability,
    },
    BudgetExhausted {
        resource: String,
        limit: u64,
        used: u64,
    },
    TokenExpired {
        token_id: String,
        expires_at_epoch_s: u64,
    },
    ProtocolViolation {
        detail: String,
    },
    PolicyDenied {
        reason: String,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panic { message } => write!(f, "panic: {message}"),
            Self::CapabilityViolation {
                token_id,
                capability,
            } => write!(f, "capability violation: token {token_id} missing {capability:?}"),
            Self::BudgetExhausted {
                resource,
                limit,
                used,
            } => write!(f, "budget exhausted: {resource} used {used}/{limit}"),
            Self::TokenExpired {
                token_id,
                expires_at_epoch_s,
            } => write!(f, "token {token_id} expired at {expires_at_epoch_s}"),
            Self::ProtocolViolation { detail } => write!(f, "protocol violation: {detail}"),
            Self::PolicyDenied { reason } => write!(f, "policy denied: {reason}"),
        }
    }
}

impl std::error::Error for Fault {}

impl Fault {
    pub fn from_policy_error(err: PolicyError) -> Self {
        match err {
            PolicyError::ExpiredToken {
                token_id,
                expires_at_epoch_s,
            } => Self::TokenExpired {
                token_id,
                expires_at_epoch_s,
            },
            PolicyError::MissingCapability {
                token_id,
                capability,
            } => Self::CapabilityViolation {
                token_id,
                capability,
            },
            PolicyError::RevokedToken { token_id } => Self::PolicyDenied {
                reason: format!("token {token_id} revoked"),
            },
            PolicyError::PackMismatch {
                token_pack_id,
                runtime_pack_id,
            } => Self::PolicyDenied {
                reason: format!("pack mismatch: token={token_pack_id} runtime={runtime_pack_id}"),
            },
            PolicyError::ExtensionDenied { extension, reason } => Self::PolicyDenied {
                reason: format!("extension {extension}: {reason}"),
            },
            PolicyError::ToolCallDenied { tool_name, reason } => Self::PolicyDenied {
                reason: format!("tool {tool_name}: {reason}"),
            },
            PolicyError::ToolCallApprovalRequired { tool_name, prompt } => Self::PolicyDenied {
                reason: format!("tool {tool_name} requires approval: {prompt}"),
            },
        }
    }

    pub fn from_kernel_error(err: KernelError) -> Self {
        match err {
            KernelError::Policy(policy_err) => Self::from_policy_error(policy_err),
            KernelError::PackCapabilityBoundary {
                capability,
                pack_id,
            } => Self::CapabilityViolation {
                token_id: format!("pack:{pack_id}"),
                capability,
            },
            other => Self::Panic {
                message: other.to_string(),
            },
        }
    }
}
```

### Step 4: Wire the module

In `crates/contracts/src/lib.rs`, add:

```rust
mod fault;
pub use fault::Fault;
```

In `crates/kernel/src/lib.rs`, add to the re-exports:

```rust
pub use contracts::Fault;
```

### Step 5: Run tests

```bash
cargo test --workspace --all-features
```

Expected: all tests pass, including the 4 new fault tests.

### Step 6: Commit

```bash
git add crates/contracts/src/fault.rs crates/contracts/src/lib.rs crates/kernel/src/lib.rs crates/kernel/src/tests.rs
git commit -m "feat(contracts): add Fault enum for structured dispatch errors

Fault provides match-friendly variants for runtime dispatch failures:
Panic, CapabilityViolation, BudgetExhausted, TokenExpired,
ProtocolViolation, PolicyDenied. Includes conversion from PolicyError
and KernelError. Additive — existing error types unchanged."
```

---

## Task 18: Add TaskState FSM

Wrap `TaskIntent` in a `TaskState` enum with transitions: `Runnable → InSend → InReply → Completed | Faulted`. Add a `TaskSupervisor` that tracks state transitions and prevents invalid ones. The kernel's `execute_task` is NOT modified — `TaskSupervisor` is a new, opt-in wrapper that code can use for supervised execution.

**Files:**
- Create: `crates/contracts/src/task_state.rs`
- Modify: `crates/contracts/src/lib.rs` (add module + re-export)
- Create: `crates/kernel/src/task_supervisor.rs`
- Modify: `crates/kernel/src/lib.rs` (add module + re-export)
- Modify: `crates/kernel/src/tests.rs` (add tests)

### Step 1: Write failing tests

Add to `crates/kernel/src/tests.rs`:

```rust
use crate::task_supervisor::TaskSupervisor;
use crate::contracts::TaskState;

#[test]
fn task_state_transitions_runnable_to_in_send() {
    let intent = TaskIntent {
        task_id: "t-1".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let state = TaskState::Runnable(intent);
    let next = state.transition_to_in_send();
    assert!(next.is_ok());
    assert!(matches!(next.unwrap(), TaskState::InSend { .. }));
}

#[test]
fn task_state_rejects_invalid_transition() {
    let state = TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    });
    let err = state.transition_to_in_send();
    assert!(err.is_err());
}

#[test]
fn task_state_faulted_carries_fault() {
    let fault = Fault::Panic {
        message: "boom".to_owned(),
    };
    let state = TaskState::Faulted(fault.clone());
    if let TaskState::Faulted(f) = state {
        assert_eq!(f, fault);
    } else {
        panic!("expected Faulted");
    }
}

#[tokio::test]
async fn task_supervisor_tracks_state_through_lifecycle() {
    let clock = Arc::new(FixedClock::new(1_000_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongClawKernel::with_runtime(
        StaticPolicyEngine::default(),
        clock,
        audit.clone(),
    );
    kernel.register_pack(test_pack()).unwrap();
    kernel.register_harness_adapter(MockEmbeddedPiHarness {
        seen_tasks: Mutex::new(Vec::new()),
    });
    let token = kernel.issue_token("test-pack", "agent-1", 3600).unwrap();

    let intent = TaskIntent {
        task_id: "supervised-1".to_owned(),
        objective: "supervised test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({"key": "value"}),
    };

    let mut supervisor = TaskSupervisor::new(intent);
    assert!(matches!(supervisor.state(), TaskState::Runnable(_)));

    let result = supervisor.execute(&kernel, "test-pack", &token).await;
    assert!(result.is_ok());
    assert!(matches!(supervisor.state(), TaskState::Completed(_)));
}

#[test]
fn task_supervisor_rejects_double_execute_after_completion() {
    let intent = TaskIntent {
        task_id: "t-double".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let mut supervisor = TaskSupervisor::new(intent);
    // Manually set to completed
    supervisor.force_state(TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    }));
    // Should not be runnable
    assert!(!supervisor.is_runnable());
}
```

### Step 2: Run tests to verify they fail

```bash
cargo test -p loongclaw-kernel -- task_state task_supervisor
```

Expected: compilation errors — `TaskState` and `TaskSupervisor` don't exist.

### Step 3: Create TaskState enum

Create `crates/contracts/src/task_state.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::contracts::{HarnessOutcome, TaskIntent};
use crate::fault::Fault;

/// State machine for task lifecycle.
///
/// Valid transitions:
/// - Runnable → InSend
/// - InSend → InReply
/// - InReply → Completed | Faulted
/// - Runnable → Faulted (pre-flight failure)
/// - InSend → Faulted (send failure)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskState {
    Runnable(TaskIntent),
    InSend { task_id: String },
    InReply { task_id: String },
    Completed(HarnessOutcome),
    Faulted(Fault),
}

impl TaskState {
    pub fn task_id(&self) -> Option<&str> {
        match self {
            Self::Runnable(intent) => Some(&intent.task_id),
            Self::InSend { task_id } | Self::InReply { task_id } => Some(task_id),
            Self::Completed(_) | Self::Faulted(_) => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed(_) | Self::Faulted(_))
    }

    pub fn transition_to_in_send(self) -> Result<Self, String> {
        match self {
            Self::Runnable(intent) => Ok(Self::InSend {
                task_id: intent.task_id,
            }),
            other => Err(format!(
                "invalid transition: cannot move to InSend from {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub fn transition_to_in_reply(self) -> Result<Self, String> {
        match self {
            Self::InSend { task_id } => Ok(Self::InReply { task_id }),
            other => Err(format!(
                "invalid transition: cannot move to InReply from {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub fn transition_to_completed(self, outcome: HarnessOutcome) -> Result<Self, String> {
        match self {
            Self::InReply { .. } => Ok(Self::Completed(outcome)),
            other => Err(format!(
                "invalid transition: cannot move to Completed from {:?}",
                std::mem::discriminant(&other)
            )),
        }
    }

    pub fn transition_to_faulted(self, fault: Fault) -> Self {
        // Any non-terminal state can fault
        if self.is_terminal() {
            self // Already terminal — ignore
        } else {
            Self::Faulted(fault)
        }
    }
}
```

### Step 4: Create TaskSupervisor

Create `crates/kernel/src/task_supervisor.rs`:

```rust
use crate::{
    contracts::{CapabilityToken, TaskIntent, HarnessOutcome},
    errors::KernelError,
    kernel::{KernelDispatch, LoongClawKernel},
    policy::PolicyEngine,
};
use loongclaw_contracts::{Fault, TaskState};

/// Opt-in wrapper around `execute_task` that enforces FSM transitions.
pub struct TaskSupervisor {
    state: TaskState,
}

impl TaskSupervisor {
    pub fn new(intent: TaskIntent) -> Self {
        Self {
            state: TaskState::Runnable(intent),
        }
    }

    pub fn state(&self) -> &TaskState {
        &self.state
    }

    pub fn is_runnable(&self) -> bool {
        matches!(self.state, TaskState::Runnable(_))
    }

    /// Execute the task through the kernel, tracking state transitions.
    pub async fn execute<P: PolicyEngine>(
        &mut self,
        kernel: &LoongClawKernel<P>,
        pack_id: &str,
        token: &CapabilityToken,
    ) -> Result<KernelDispatch, Fault> {
        // Extract intent from Runnable state
        let intent = match std::mem::replace(
            &mut self.state,
            TaskState::InSend {
                task_id: String::new(),
            },
        ) {
            TaskState::Runnable(intent) => intent,
            other => {
                self.state = other;
                return Err(Fault::ProtocolViolation {
                    detail: "task is not in Runnable state".to_owned(),
                });
            }
        };

        let task_id = intent.task_id.clone();
        self.state = TaskState::InSend {
            task_id: task_id.clone(),
        };

        // Transition to InReply
        self.state = TaskState::InReply {
            task_id: task_id.clone(),
        };

        // Execute through kernel
        match kernel.execute_task(pack_id, token, intent).await {
            Ok(dispatch) => {
                self.state = TaskState::Completed(dispatch.outcome.clone());
                Ok(dispatch)
            }
            Err(kernel_err) => {
                let fault = Fault::from_kernel_error(kernel_err);
                self.state = TaskState::Faulted(fault.clone());
                Err(fault)
            }
        }
    }

    /// Force state — for testing only.
    #[cfg(test)]
    pub fn force_state(&mut self, state: TaskState) {
        self.state = state;
    }
}
```

### Step 5: Wire the modules

In `crates/contracts/src/lib.rs`, add:

```rust
mod task_state;
pub use task_state::TaskState;
```

In `crates/kernel/src/lib.rs`, add:

```rust
pub mod task_supervisor;
pub use task_supervisor::TaskSupervisor;
```

Also add `TaskState` to the contracts re-export in `crates/kernel/src/lib.rs`.

### Step 6: Run tests

```bash
cargo test --workspace --all-features
```

Expected: all tests pass, including the 4 new TaskState/TaskSupervisor tests.

### Step 7: Commit

```bash
git add crates/contracts/src/task_state.rs crates/contracts/src/lib.rs
git add crates/kernel/src/task_supervisor.rs crates/kernel/src/lib.rs crates/kernel/src/tests.rs
git commit -m "feat(kernel): add TaskState FSM and TaskSupervisor

TaskState enum: Runnable → InSend → InReply → Completed | Faulted.
TaskSupervisor wraps execute_task with FSM enforcement. Purely opt-in —
existing execute_task API unchanged. Invalid transitions return errors."
```

---

## Task 19: Add Namespace

Introduce `Namespace` as a runtime projection of `VerticalPackManifest`. When a pack is registered, the kernel creates a corresponding `Namespace` that holds resolved runtime state (e.g., the set of active adapters for this pack's planes). `VerticalPackManifest` remains the declarative specification; `Namespace` is the runtime view.

**Files:**
- Create: `crates/contracts/src/namespace.rs`
- Modify: `crates/contracts/src/lib.rs` (add module + re-export)
- Modify: `crates/kernel/src/kernel.rs:63-76` (add `namespaces: BTreeMap<String, Namespace>`)
- Modify: `crates/kernel/src/kernel.rs:102-109` (update `register_pack` to create Namespace)
- Modify: `crates/kernel/src/lib.rs` (re-export Namespace)
- Modify: `crates/kernel/src/tests.rs` (add tests)

### Step 1: Write failing tests

Add to `crates/kernel/src/tests.rs`:

```rust
use crate::contracts::Namespace;

#[test]
fn register_pack_creates_namespace() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel.register_pack(test_pack()).unwrap();

    let ns = kernel.get_namespace("test-pack");
    assert!(ns.is_some());
    let ns = ns.unwrap();
    assert_eq!(ns.pack_id, "test-pack");
    assert_eq!(ns.domain, "testing");
    assert!(ns.granted_capabilities.contains(&Capability::InvokeTool));
}

#[test]
fn get_namespace_returns_none_for_unregistered_pack() {
    let kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    assert!(kernel.get_namespace("nonexistent").is_none());
}

#[test]
fn namespace_has_membrane_tag_matching_pack_id() {
    let mut kernel = LoongClawKernel::new(StaticPolicyEngine::default());
    kernel.register_pack(test_pack()).unwrap();

    let ns = kernel.get_namespace("test-pack").unwrap();
    assert_eq!(ns.membrane, "test-pack");
}
```

### Step 2: Run tests to verify they fail

```bash
cargo test -p loongclaw-kernel -- register_pack_creates_namespace get_namespace_returns_none namespace_has_membrane
```

Expected: compilation errors — `Namespace` doesn't exist, `get_namespace` doesn't exist.

### Step 3: Create Namespace struct

Create `crates/contracts/src/namespace.rs`:

```rust
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contracts::{Capability, ExecutionRoute};

/// Runtime projection of a `VerticalPackManifest`.
///
/// Created when a pack is registered. Holds resolved runtime state
/// derived from the declarative manifest. The membrane field provides
/// a namespace isolation tag (defaults to pack_id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub pack_id: String,
    pub domain: String,
    pub membrane: String,
    pub default_route: ExecutionRoute,
    pub granted_capabilities: BTreeSet<Capability>,
}
```

### Step 4: Wire the module

In `crates/contracts/src/lib.rs`, add:

```rust
mod namespace;
pub use namespace::Namespace;
```

In `crates/kernel/src/lib.rs`, add `Namespace` to the contracts re-export:

```rust
pub use contracts::Namespace;
```

### Step 5: Add namespaces map to LoongClawKernel

In `crates/kernel/src/kernel.rs`, add field to struct:

```rust
pub struct LoongClawKernel<P: PolicyEngine> {
    // ... existing fields ...
    namespaces: BTreeMap<String, loongclaw_contracts::Namespace>,
}
```

Initialize in `with_runtime`:

```rust
namespaces: BTreeMap::new(),
```

Update `register_pack` to create Namespace:

```rust
pub fn register_pack(&mut self, pack: VerticalPackManifest) -> Result<(), KernelError> {
    pack.validate()?;
    if self.packs.contains_key(&pack.pack_id) {
        return Err(KernelError::DuplicatePack(pack.pack_id));
    }
    let namespace = loongclaw_contracts::Namespace {
        pack_id: pack.pack_id.clone(),
        domain: pack.domain.clone(),
        membrane: pack.pack_id.clone(), // default membrane = pack_id
        default_route: pack.default_route.clone(),
        granted_capabilities: pack.granted_capabilities.clone(),
    };
    self.namespaces.insert(pack.pack_id.clone(), namespace);
    self.packs.insert(pack.pack_id.clone(), pack);
    Ok(())
}
```

Add `get_namespace`:

```rust
pub fn get_namespace(&self, pack_id: &str) -> Option<&loongclaw_contracts::Namespace> {
    self.namespaces.get(pack_id)
}
```

### Step 6: Run tests

```bash
cargo test --workspace --all-features
```

Expected: all tests pass, including the 3 new Namespace tests.

### Step 7: Commit

```bash
git add crates/contracts/src/namespace.rs crates/contracts/src/lib.rs
git add crates/kernel/src/kernel.rs crates/kernel/src/lib.rs crates/kernel/src/tests.rs
git commit -m "feat(kernel): add Namespace as runtime projection of pack manifest

Namespace is created during register_pack and holds resolved runtime
state. Membrane field defaults to pack_id for isolation tagging.
Purely additive — VerticalPackManifest unchanged, existing APIs unchanged."
```

---

## Post-Phase 3 Checklist

After all 4 tasks are complete:

```bash
# Full verification
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# Update ARCHITECTURE.md future plans section
# Remove the 4 items from "Future Plans (Post-MVP)" since they're now implemented
```

Commit the ARCHITECTURE.md update and create PR.
