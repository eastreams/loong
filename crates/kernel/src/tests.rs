use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proptest::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::audit::{
    AuditEvent, AuditEventKind, AuditRepairOutcome, AuditSink, FanoutAuditSink, InMemoryAuditSink,
    JsonlAuditSink, probe_jsonl_audit_journal_runtime_ready, repair_jsonl_audit_journal,
    verify_jsonl_audit_journal,
};
use crate::clock::FixedClock;
use crate::contracts::{Capability, HarnessOutcome, TaskIntent};
use crate::errors::{AuditError, KernelError, PolicyError};
use crate::kernel::LoongKernel;
use crate::policy::{PolicyEngine, StaticPolicyEngine};
use crate::task_supervisor::TaskSupervisor;
use crate::{ExecutionPlane, PlaneTier};
use crate::{Fault, TaskState};

use crate::test_support::*;

fn fresh_audit_temp_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!(
        "loong-kernel-audit-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn sample_audit_event(event_id: &str, timestamp_epoch_s: u64) -> AuditEvent {
    AuditEvent {
        event_id: event_id.to_owned(),
        timestamp_epoch_s,
        agent_id: Some("agent-audit".to_owned()),
        kind: AuditEventKind::PlaneInvoked {
            pack_id: "sales-intel".to_owned(),
            plane: ExecutionPlane::Tool,
            tier: PlaneTier::Core,
            primary_adapter: "mvp-tools".to_owned(),
            delegated_core_adapter: None,
            operation: "shell.exec".to_owned(),
            required_capabilities: vec![Capability::InvokeTool],
        },
    }
}

fn compute_test_entry_hash(event: &AuditEvent, prev_hash: Option<&str>) -> String {
    let material = serde_json::to_vec(&json!({
        "event_id": event.event_id,
        "timestamp_epoch_s": event.timestamp_epoch_s,
        "agent_id": event.agent_id,
        "kind": event.kind,
        "prev_hash": prev_hash,
    }))
    .expect("serialize audit chain material");
    let digest = Sha256::digest(material);
    hex::encode(digest)
}

#[test]
fn jsonl_audit_sink_appends_one_json_line_per_event() {
    let path = fresh_audit_temp_path("jsonl");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-1", 100))
        .expect("first event should record");
    sink.record(sample_audit_event("evt-2", 101))
        .expect("second event should record");

    let contents = fs::read_to_string(&path).expect("audit journal should exist");
    let lines = contents.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "expected one JSON line per audit event");

    let first: AuditEvent =
        serde_json::from_str(lines[0]).expect("first JSON line should decode into AuditEvent");
    let second: AuditEvent =
        serde_json::from_str(lines[1]).expect("second JSON line should decode into AuditEvent");
    let first_payload = serde_json::from_str::<serde_json::Value>(lines[0])
        .expect("first JSON line should decode into a JSON payload");
    let second_payload = serde_json::from_str::<serde_json::Value>(lines[1])
        .expect("second JSON line should decode into a JSON payload");
    assert_eq!(first.event_id, "evt-1");
    assert_eq!(second.event_id, "evt-2");
    assert!(first_payload.get("integrity").is_some());
    assert!(second_payload.get("integrity").is_some());

    let _ = fs::remove_file(path);
}

#[test]
fn verify_jsonl_audit_journal_accepts_freshly_written_chain() {
    let path = fresh_audit_temp_path("jsonl-verify");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-verify-1", 300))
        .expect("first event should record");
    sink.record(sample_audit_event("evt-verify-2", 301))
        .expect("second event should record");

    let report = verify_jsonl_audit_journal(&path).expect("verification should succeed");

    assert!(report.valid);
    assert_eq!(report.total_events, 2);
    assert_eq!(report.verified_events, 2);
    assert!(report.last_entry_hash.is_some());

    let _ = fs::remove_file(path);
}

#[test]
fn verify_jsonl_audit_journal_rejects_tampered_chain_entry() {
    let path = fresh_audit_temp_path("jsonl-tamper");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-tamper-1", 400))
        .expect("first event should record");
    sink.record(sample_audit_event("evt-tamper-2", 401))
        .expect("second event should record");

    let contents = fs::read_to_string(&path).expect("read audit journal");
    let tampered = contents.replacen("evt-tamper-2", "evt-tamper-x", 1);
    fs::write(&path, tampered).expect("rewrite tampered audit journal");

    let report = verify_jsonl_audit_journal(&path).expect("verification should run");

    assert!(!report.valid);
    assert_eq!(report.first_invalid_line, Some(2));
    assert_eq!(report.reason.as_deref(), Some("entry_hash mismatch"));

    let _ = fs::remove_file(path);
}

#[test]
fn verify_jsonl_audit_journal_accepts_legacy_prefix_before_protected_entries() {
    let path = fresh_audit_temp_path("jsonl-legacy-prefix");
    let legacy_event = sample_audit_event("evt-legacy-1", 500);
    let legacy_line = serde_json::to_string(&legacy_event).expect("serialize legacy audit event");
    let legacy_contents = format!("{legacy_line}\n");

    fs::write(&path, legacy_contents).expect("write legacy audit journal");

    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-verify-legacy-tail", 501))
        .expect("protected event should record");

    let report = verify_jsonl_audit_journal(&path).expect("verification should succeed");

    assert!(report.valid);
    assert_eq!(report.total_events, 2);
    assert_eq!(report.verified_events, 1);
    assert!(report.last_entry_hash.is_some());

    let _ = fs::remove_file(path);
}

#[test]
fn repair_jsonl_audit_journal_reports_healthy_for_valid_chain() {
    let path = fresh_audit_temp_path("jsonl-repair-healthy");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-repair-healthy-1", 600))
        .expect("first event should record");
    sink.record(sample_audit_event("evt-repair-healthy-2", 601))
        .expect("second event should record");

    let original_contents = fs::read_to_string(&path).expect("read original journal");
    let report = repair_jsonl_audit_journal(&path).expect("repair should succeed");
    let repaired_contents = fs::read_to_string(&path).expect("read repaired journal");

    assert_eq!(report.total_events, 2);
    assert_eq!(report.repaired_events, 0);
    assert_eq!(report.already_valid_events, 2);
    assert_eq!(report.outcome, AuditRepairOutcome::Healthy);
    assert_eq!(repaired_contents, original_contents);

    let _ = fs::remove_file(path);
}

#[test]
fn repair_jsonl_audit_journal_repairs_legacy_prefix_and_reseals_tail() {
    let path = fresh_audit_temp_path("jsonl-repair-legacy-prefix");
    let legacy_event = sample_audit_event("evt-repair-legacy-1", 700);
    let legacy_line = serde_json::to_string(&legacy_event).expect("serialize legacy event");
    let legacy_contents = format!("{legacy_line}\n");

    fs::write(&path, legacy_contents).expect("write legacy journal");

    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-repair-tail-1", 701))
        .expect("protected event should record");

    let report = repair_jsonl_audit_journal(&path).expect("repair should succeed");
    let verify_report = verify_jsonl_audit_journal(&path).expect("verify repaired journal");

    assert_eq!(report.total_events, 2);
    assert_eq!(report.repaired_events, 2);
    assert_eq!(report.already_valid_events, 0);
    assert_eq!(report.outcome, AuditRepairOutcome::Repaired);
    assert!(verify_report.valid);
    assert_eq!(verify_report.verified_events, 2);

    let _ = fs::remove_file(path);
}

#[test]
fn repair_jsonl_audit_journal_refuses_tampered_entry_hash() {
    let path = fresh_audit_temp_path("jsonl-repair-tamper");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");

    sink.record(sample_audit_event("evt-repair-tamper-1", 800))
        .expect("first event should record");
    sink.record(sample_audit_event("evt-repair-tamper-2", 801))
        .expect("second event should record");

    let original_contents = fs::read_to_string(&path).expect("read audit journal");
    let tampered_contents =
        original_contents.replacen("evt-repair-tamper-2", "evt-repair-tamper-x", 1);

    fs::write(&path, &tampered_contents).expect("rewrite tampered audit journal");

    let report = repair_jsonl_audit_journal(&path).expect("repair should report refusal");
    let after_contents = fs::read_to_string(&path).expect("read journal after refusal");

    assert_eq!(report.total_events, 2);
    assert_eq!(report.repaired_events, 0);
    assert_eq!(report.already_valid_events, 1);
    assert_eq!(
        report.outcome,
        AuditRepairOutcome::Refused {
            line: 2,
            reason: "entry_hash mismatch — event data may be tampered".to_owned(),
        }
    );
    assert_eq!(after_contents, tampered_contents);

    let _ = fs::remove_file(path);
}

#[test]
fn repair_jsonl_audit_journal_refuses_first_protected_row_with_prev_hash() {
    let path = fresh_audit_temp_path("jsonl-repair-prev-hash");
    let event = sample_audit_event("evt-repair-prev-hash-1", 900);
    let prev_hash = "unexpected-prev-hash".to_owned();
    let entry_hash = compute_test_entry_hash(&event, Some(&prev_hash));
    let encoded = serde_json::to_string(&json!({
        "event_id": event.event_id,
        "timestamp_epoch_s": event.timestamp_epoch_s,
        "agent_id": event.agent_id,
        "kind": event.kind,
        "integrity": {
            "algorithm": "sha256",
            "prev_hash": prev_hash,
            "entry_hash": entry_hash,
        },
    }))
    .expect("serialize protected audit event");

    fs::write(&path, format!("{encoded}\n")).expect("write forged journal");

    let report = repair_jsonl_audit_journal(&path).expect("repair should report refusal");

    assert_eq!(report.total_events, 1);
    assert_eq!(report.repaired_events, 0);
    assert_eq!(report.already_valid_events, 0);
    assert_eq!(
        report.outcome,
        AuditRepairOutcome::Refused {
            line: 1,
            reason: "prev_hash mismatch in source chain".to_owned(),
        }
    );

    let _ = fs::remove_file(path);
}

#[test]
fn fanout_audit_sink_records_to_all_children() {
    let path = fresh_audit_temp_path("fanout");
    let memory = Arc::new(InMemoryAuditSink::default());
    let jsonl = Arc::new(JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize"));
    let sink = FanoutAuditSink::new(vec![
        memory.clone() as Arc<dyn AuditSink>,
        jsonl as Arc<dyn AuditSink>,
    ]);

    sink.record(sample_audit_event("evt-fanout", 200))
        .expect("fanout sink should record");

    let memory_events = memory.snapshot();
    assert_eq!(
        memory_events.len(),
        1,
        "in-memory sink should receive event"
    );

    let contents = fs::read_to_string(&path).expect("jsonl fanout sink should write file");
    assert_eq!(
        contents.lines().count(),
        1,
        "jsonl child should receive event"
    );

    let _ = fs::remove_file(path);
}

#[test]
fn explicit_in_memory_kernel_constructor_records_token_audit_events() {
    let (mut kernel, audit) = LoongKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");

    kernel
        .issue_token("sales-intel", "agent-in-memory", 120)
        .expect("token should issue");

    let events = audit.snapshot();
    assert_eq!(events.len(), 1, "expected one token-issued audit event");
    assert!(matches!(events[0].kind, AuditEventKind::TokenIssued { .. }));
}

#[test]
fn explicit_no_audit_kernel_constructor_keeps_side_effect_free_fixture_path() {
    let mut kernel = LoongKernel::new_without_audit(StaticPolicyEngine::default());
    kernel
        .register_pack(sample_pack())
        .expect("pack should register");

    let token = kernel
        .issue_token("sales-intel", "agent-no-audit", 120)
        .expect("token should issue without wiring an audit sink");

    assert_eq!(token.agent_id, "agent-no-audit");
}

#[test]
fn jsonl_audit_sink_surfaces_io_errors() {
    let path = fresh_audit_temp_path("jsonl-dir");
    fs::create_dir(&path).expect("directory fixture should create");

    let error = JsonlAuditSink::new(path.clone()).expect_err("directory path should fail");
    assert!(matches!(error, AuditError::Sink(_)));

    let _ = fs::remove_dir_all(path);
}

#[test]
fn jsonl_audit_sink_runtime_probe_accepts_fresh_journal_path() {
    let path = fresh_audit_temp_path("jsonl-probe");

    probe_jsonl_audit_journal_runtime_ready(&path)
        .expect("runtime readiness probe should succeed for a fresh journal path");

    let _ = fs::remove_file(path);
}

#[test]
fn jsonl_audit_sink_waits_for_existing_file_lock_before_appending() {
    let path = fresh_audit_temp_path("jsonl-lock");
    let sink = JsonlAuditSink::new(path.clone()).expect("jsonl sink should initialize");
    let external_lock = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&path)
        .expect("open external audit journal handle");
    external_lock
        .lock()
        .expect("hold external audit journal lock");

    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let result = sink.record(sample_audit_event("evt-locked", 202));
        tx.send(result).expect("send audit result");
    });

    match rx.recv_timeout(Duration::from_millis(100)) {
        Err(mpsc::RecvTimeoutError::Timeout) => {}
        Ok(result) => panic!("audit write should block on external file lock, got {result:?}"),
        Err(error) => panic!("audit write channel closed unexpectedly: {error:?}"),
    }

    external_lock
        .unlock()
        .expect("release external audit journal lock");
    rx.recv_timeout(Duration::from_secs(1))
        .expect("audit write should complete after lock release")
        .expect("audit write should succeed after lock release");
    handle.join().expect("join audit writer thread");

    let contents = fs::read_to_string(&path).expect("audit journal should exist");
    assert_eq!(contents.lines().count(), 1);

    let _ = fs::remove_file(path);
}

#[test]
#[should_panic(expected = "fanout audit sink requires at least one child")]
fn fanout_audit_sink_rejects_empty_children() {
    let _ = FanoutAuditSink::new(Vec::new());
}

#[test]
fn pack_validation_rejects_invalid_semver() {
    let mut pack = sample_pack();
    pack.version = "version-one".to_owned();

    let error = pack.validate().expect_err("invalid semver should fail");
    assert!(matches!(error, crate::PackError::InvalidVersion(_)));
}

#[test]
fn token_generation_increments_on_each_issue() {
    let engine = StaticPolicyEngine::default();
    let pack = sample_pack();
    let t1 = engine.issue_token(&pack, "a1", 1_000_000, 3600).unwrap();
    let t2 = engine.issue_token(&pack, "a2", 1_000_000, 3600).unwrap();
    let t3 = engine.issue_token(&pack, "a3", 1_000_000, 3600).unwrap();
    assert_eq!(t1.generation, 1);
    assert_eq!(t2.generation, 2);
    assert_eq!(t3.generation, 3);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_pack_capability_boundary_for_task_dispatch(
        pack_mask in 1_u16..(1_u16 << TEST_CAPABILITY_VARIANT_COUNT),
        required_mask in 0_u16..(1_u16 << TEST_CAPABILITY_VARIANT_COUNT)
    ) {
        let pack_capabilities = capability_set_from_mask(pack_mask);
        let required_capabilities = capability_set_from_mask(required_mask);

        let (mut kernel, _audit) =
            LoongKernel::new_with_in_memory_audit(StaticPolicyEngine::default());
        let mut pack = sample_pack();
        pack.granted_capabilities = pack_capabilities.clone();
        kernel
            .register_pack(pack)
            .expect("pack should register");
        kernel.register_harness_adapter(MockEmbeddedPiHarness {
            seen_tasks: Mutex::new(Vec::new()),
        });

        let token = kernel
            .issue_token("sales-intel", "agent-prop", 120)
            .expect("token should issue");

        let task = TaskIntent {
            task_id: "task-prop".to_owned(),
            objective: "property boundary check".to_owned(),
            required_capabilities: required_capabilities.clone(),
            payload: json!({}),
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime should build");
        let result = runtime.block_on(kernel.execute_task("sales-intel", &token, task));

        if required_capabilities.is_subset(&pack_capabilities) {
            prop_assert!(result.is_ok());
        } else {
            let boundary_error = matches!(result, Err(KernelError::PackCapabilityBoundary { .. }));
            prop_assert!(boundary_error);
        }
    }
}

// ---------------------------------------------------------------------------
// Fault enum tests
// ---------------------------------------------------------------------------

#[test]
fn fault_display_is_human_readable() {
    let fault = Fault::CapabilityViolation {
        token_id: "tok-1".to_owned(),
        capability: Capability::InvokeTool,
    };
    let msg = fault.to_string();
    assert!(msg.contains("tok-1"));
    assert!(msg.contains("InvokeTool"));
}

#[test]
fn fault_from_policy_error_maps_expired_token() {
    let policy_err = PolicyError::ExpiredToken {
        token_id: "tok-2".to_owned(),
        expires_at_epoch_s: 1000,
    };
    let fault = Fault::from_policy_error(policy_err);
    assert!(
        matches!(fault, Fault::TokenExpired { token_id, expires_at_epoch_s } if token_id == "tok-2" && expires_at_epoch_s == 1000)
    );
}

#[test]
fn fault_from_policy_error_maps_missing_capability() {
    let policy_err = PolicyError::MissingCapability {
        token_id: "tok-3".to_owned(),
        capability: Capability::MemoryWrite,
    };
    let fault = Fault::from_policy_error(policy_err);
    assert!(matches!(fault, Fault::CapabilityViolation { .. }));
}

#[test]
fn fault_from_kernel_error_maps_policy() {
    let kernel_err = KernelError::Policy(PolicyError::RevokedToken {
        token_id: "tok-4".to_owned(),
    });
    let fault = Fault::from_kernel_error(kernel_err);
    assert!(matches!(fault, Fault::PolicyDenied { .. }));
}

#[test]
fn fault_from_kernel_error_maps_pack_boundary() {
    let kernel_err = KernelError::PackCapabilityBoundary {
        pack_id: "my-pack".to_owned(),
        capability: Capability::NetworkEgress,
    };
    let fault = Fault::from_kernel_error(kernel_err);
    assert!(matches!(fault, Fault::CapabilityViolation { .. }));
}

#[test]
fn fault_panic_carries_message() {
    let fault = Fault::Panic {
        message: "unexpected state".to_owned(),
    };
    assert!(fault.to_string().contains("unexpected state"));
}

// ── TaskState FSM tests ──────────────────────────────────────────────

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
fn task_state_rejects_invalid_transition_from_completed() {
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

#[test]
fn task_state_full_transition_chain() {
    let intent = TaskIntent {
        task_id: "t-chain".to_owned(),
        objective: "chain test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let state = TaskState::Runnable(intent);
    let state = state.transition_to_in_send().unwrap();
    assert!(matches!(state, TaskState::InSend { .. }));
    let state = state.transition_to_in_reply().unwrap();
    assert!(matches!(state, TaskState::InReply { .. }));
    let outcome = HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({"result": "done"}),
    };
    let state = state.transition_to_completed(outcome).unwrap();
    assert!(matches!(state, TaskState::Completed(_)));
    assert!(state.is_terminal());
}

#[test]
fn task_state_faulted_from_non_terminal_succeeds() {
    let state = TaskState::InSend {
        task_id: "t-fault".to_owned(),
    };
    let fault = Fault::Panic {
        message: "oops".to_owned(),
    };
    let state = state.transition_to_faulted(fault);
    assert!(matches!(state, TaskState::Faulted(_)));
}

#[test]
fn task_state_faulted_from_terminal_is_noop() {
    let state = TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    });
    let fault = Fault::Panic {
        message: "late".to_owned(),
    };
    let state = state.transition_to_faulted(fault);
    // Should remain Completed, not change to Faulted
    assert!(matches!(state, TaskState::Completed(_)));
}

#[test]
fn task_supervisor_rejects_execute_after_completion() {
    let intent = TaskIntent {
        task_id: "t-double".to_owned(),
        objective: "test".to_owned(),
        required_capabilities: BTreeSet::from([Capability::InvokeTool]),
        payload: json!({}),
    };
    let mut supervisor = TaskSupervisor::new(intent);
    supervisor.force_state(TaskState::Completed(HarnessOutcome {
        status: "ok".to_owned(),
        output: json!({}),
    }));
    assert!(!supervisor.is_runnable());
}

#[test]
fn record_tool_call_denial_audits_extension_denied_errors() {
    let clock: Arc<FixedClock> = Arc::new(FixedClock::new(1_700_004_000));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = LoongKernel::with_runtime(StaticPolicyEngine::default(), clock, audit.clone());
    let pack = sample_pack();
    kernel
        .register_pack(pack.clone())
        .expect("pack should register");
    let token = kernel
        .issue_token(&pack.pack_id, "agent-extension-denied", 120)
        .expect("token should issue");
    let error = PolicyError::ExtensionDenied {
        extension: "policy".to_owned(),
        reason: "unexpected policy decision for tool `shell.exec`".to_owned(),
    };

    kernel
        .record_tool_call_denial(&pack, &token, 1_700_004_000, &error)
        .expect("audit record should succeed");

    let events = audit.snapshot();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[1].kind,
        AuditEventKind::AuthorizationDenied { pack_id, token_id, reason }
            if pack_id == &pack.pack_id
                && token_id == &token.token_id
                && reason.contains("unexpected policy decision for tool `shell.exec`")
    ));
}
