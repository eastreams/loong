use std::fs;
use std::sync::Arc;

use loong_contracts::{AuthorizationScope, GovernedSessionMode};
use loong_core::policy::context::PolicyContext;
use loong_kernel::InMemoryAuditSink;
use tempfile::tempdir;

use super::{bootstrap_runtime_with_audit_sink, bootstrap_runtime_with_config};
use crate::config::{AuditMode, LoongConfig, MemoryProfile};
use crate::memory::runtime_config::MemoryRuntimeConfig;
use crate::test_utils::ScopedEnv;
use crate::{Context, Session};

#[test]
fn bootstrap_does_not_issue_a_host_token() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let config = LoongConfig::default();
    let tool_rt = crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);

    let runtime = bootstrap_runtime_with_audit_sink(audit.clone(), &config, &tool_rt)
        .expect("runtime bootstrap");

    assert!(
        runtime
            .legacy_kernel()
            .pack_manifest(crate::legacy_kernel::EMBEDDED_RUNTIME_PACK_ID)
            .is_ok()
    );
    assert!(
        audit.snapshot().is_empty(),
        "runtime ownership must not mint a host-level token"
    );
}

#[test]
fn legacy_token_issuance_writes_jsonl_audit_events() {
    let tempdir = tempdir().expect("tempdir");
    let audit_path = tempdir.path().join("audit").join("events.jsonl");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::Jsonl;
    config.audit.path = audit_path.display().to_string();
    config.audit.retain_in_memory = false;

    let runtime = bootstrap_runtime_with_config(&config).expect("runtime bootstrap");
    let session = Session::from_config(
        runtime.as_ref(),
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("session");
    let token = crate::legacy_kernel::issue_session_token(runtime.as_ref(), &session, 60)
        .expect("legacy token issue");
    let context = Context::new(&runtime, &session)
        .expect("configured Session must remain bound to its Runtime");

    assert_eq!(context.agent_id(), "test-agent");
    assert!(matches!(
        context.authorization_subject().scope,
        AuthorizationScope::Session { ref session_id }
            if session_id == "test-session"
    ));
    assert_eq!(
        token.pack_id,
        crate::legacy_kernel::EMBEDDED_RUNTIME_PACK_ID
    );

    let journal = fs::read_to_string(&audit_path).expect("audit journal should exist");
    assert_eq!(
        journal.lines().count(),
        1,
        "token bootstrap should emit one audit event"
    );
    assert!(
        journal.contains("\"TokenIssued\"") || journal.contains("\"token_id\""),
        "bootstrap journal should capture token issuance"
    );
}

#[test]
fn legacy_token_issuance_writes_fanout_audit_events() {
    let tempdir = tempdir().expect("tempdir");
    let audit_path = tempdir.path().join("audit").join("events.jsonl");
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::Fanout;
    config.audit.path = audit_path.display().to_string();
    config.audit.retain_in_memory = true;

    let runtime = bootstrap_runtime_with_config(&config).expect("runtime bootstrap");
    let session = Session::from_config(
        runtime.as_ref(),
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("session");
    crate::legacy_kernel::issue_session_token(runtime.as_ref(), &session, 60)
        .expect("legacy token issue");
    let context = Context::new(&runtime, &session)
        .expect("configured Session must remain bound to its Runtime");

    assert_eq!(context.agent_id(), "test-agent");

    let journal = fs::read_to_string(&audit_path).expect("audit journal should exist");
    assert_eq!(
        journal.lines().count(),
        1,
        "token bootstrap should emit one audit event"
    );
    assert!(
        journal.contains("\"TokenIssued\"") || journal.contains("\"token_id\""),
        "fanout journal should capture token issuance"
    );
}

#[cfg(feature = "memory-sqlite")]
#[tokio::test]
async fn runtime_bootstrap_ignores_memory_env_overrides() {
    let tempdir = tempdir().expect("tempdir");
    let sqlite_path = tempdir.path().join("memory.sqlite3");

    let mut seeded_runtime = MemoryRuntimeConfig::for_sqlite_path(sqlite_path.clone());
    seeded_runtime.profile = MemoryProfile::WindowPlusSummary;
    seeded_runtime.sliding_window = 2;

    crate::memory::append_turn_direct(
        "kernel-bootstrap-env-session",
        "user",
        "turn 1",
        &seeded_runtime,
    )
    .expect("append turn 1");
    crate::memory::append_turn_direct(
        "kernel-bootstrap-env-session",
        "assistant",
        "turn 2",
        &seeded_runtime,
    )
    .expect("append turn 2");
    crate::memory::append_turn_direct(
        "kernel-bootstrap-env-session",
        "user",
        "turn 3",
        &seeded_runtime,
    )
    .expect("append turn 3");

    let mut env = ScopedEnv::new();
    env.set("LOONG_MEMORY_PROFILE", "window_plus_summary");
    env.set("LOONG_SQLITE_PATH", "/tmp/env-bootstrap-memory.sqlite3");

    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;
    config.memory.profile = MemoryProfile::WindowOnly;
    config.memory.sqlite_path = sqlite_path.display().to_string();
    config.memory.sliding_window = 2;
    crate::test_support::ensure_root_session_for_test(&config, "kernel-bootstrap-env-session")
        .expect("persist runtime memory test Session identity");

    let runtime = bootstrap_runtime_with_config(&config).expect("runtime bootstrap");
    let session = Session::from_config(
        runtime.as_ref(),
        &config,
        "kernel-bootstrap-env-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("memory session");
    let context = Context::new(runtime.as_ref(), &session).expect("memory context");
    let envelope = context
        .access()
        .memory()
        .read_stage_envelope()
        .await
        .expect("read typed stage envelope");

    assert!(
        envelope
            .hydrated
            .entries
            .iter()
            .all(|entry| entry.kind != crate::memory::MemoryContextKind::Summary),
        "window-only bootstrap should ignore env-driven summary profile"
    );
}
