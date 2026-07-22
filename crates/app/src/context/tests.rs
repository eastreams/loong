use std::fs;
use std::sync::Arc;

use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
use loong_kernel::InMemoryAuditSink;
use tempfile::tempdir;

use super::*;
use crate::config::LoongConfig;
use crate::runtime::{bootstrap_runtime_with_audit_sink, bootstrap_runtime_with_config};

// Contracts tests path validation; Context fixtures use valid registered
// identities and focus on recursive authority derivation.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

#[test]
fn advisory_session_context_uses_non_mutating_capabilities() {
    let audit = Arc::new(InMemoryAuditSink::default());
    let config = LoongConfig::default();
    let tool_runtime_config =
        crate::tools::runtime_config::ToolRuntimeConfig::from_loong_config(&config, None);
    let runtime = bootstrap_runtime_with_audit_sink(audit, &config, &tool_runtime_config)
        .expect("runtime bootstrap");

    let session = Session::from_config(
        runtime.as_ref(),
        &config,
        "advisory-session",
        "advisory-agent",
        GovernedSessionMode::AdvisoryOnly,
    )
    .expect("advisory session");
    let context = Context::new(&runtime, &session)
        .expect("configured Session must remain bound to its Runtime");

    assert_eq!(
        context.allowed_capabilities().as_ref(),
        &Capabilities::from([
            Capability::MemoryRead,
            Capability::FilesystemRead,
            Capability::NetworkEgress,
        ])
    );
    assert!(matches!(
        context.authorization_subject().scope,
        AuthorizationScope::Session { ref session_id }
            if session_id == "advisory-session"
    ));
}

#[test]
fn authorization_subject_reads_the_owned_session_identity() {
    let owner = crate::test_support::runtime_session_for_test(
        "session-identity",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();

    assert!(matches!(
        context.authorization_subject().scope,
        AuthorizationScope::Session { ref session_id }
            if session_id == "session-identity"
    ));
}

#[test]
fn tool_child_narrows_policy_capabilities() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let narrowed = Capabilities::from([Capability::MemoryRead]);

    let execution_context = context
        .derive_tool_child(narrowed.clone())
        .expect("narrowed execution context should build");

    assert_eq!(execution_context.allowed_capabilities().as_ref(), &narrowed);
    assert!(
        context
            .allowed_capabilities()
            .contains(Capability::InvokeTool)
    );
}

#[test]
fn tool_child_rejects_added_capabilities() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let widened = Capabilities::from([Capability::MemoryRead, Capability::ControlRead]);

    let error = match context.derive_tool_child(widened) {
        Ok(_) => panic!("execution context must not add capabilities"),
        Err(error) => error,
    };

    assert!(error.derived.contains(Capability::ControlRead));
    assert!(!error.allowed.contains(Capability::ControlRead));
}

#[test]
fn tool_child_rejects_capabilities_removed_by_parent_context() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let parent = context
        .derive_tool_child(Capabilities::from([Capability::MemoryRead]))
        .expect("parent execution context should build");
    let child_caps = Capabilities::from([Capability::MemoryRead, Capability::FilesystemRead]);

    let error = match parent.derive_tool_child(child_caps.clone()) {
        Ok(_) => panic!("child context must not regain parent-removed capabilities"),
        Err(error) => error,
    };

    assert_eq!(error.derived, child_caps);
    assert_eq!(error.allowed, Capabilities::from([Capability::MemoryRead]));
}

#[test]
fn recursive_session_rebind_preserves_the_current_capability_ceiling() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let base = owner.context();
    let narrowed = base
        .derive_tool_child(Capabilities::from([Capability::MemoryRead]))
        .expect("narrow recursive context");
    let rematerialized = owner.session.clone();

    let rebound = narrowed
        .rebind_session(&rematerialized)
        .expect("rebind the same Session");

    assert_eq!(
        rebound.allowed_capabilities().as_ref(),
        &Capabilities::from([Capability::MemoryRead])
    );
}

#[test]
fn recursive_session_rebind_rejects_same_session_with_wider_tool_view() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::ToolView::from_legacy_paths(["read"]),
    );
    let context = owner.context();
    let mut widened = owner.session.clone();
    widened.tool_view = crate::tools::runtime_tool_view();

    let error = context
        .rebind_session(&widened)
        .expect_err("same-id Session must not restore hidden tools");

    assert!(matches!(
        error,
        ContextSessionError::AuthorityExpanded {
            dimension: "tool visibility"
        }
    ));
}

#[test]
fn recursive_session_rebind_rejects_same_session_with_wider_runtime_config() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let mut widened = owner.session.clone();
    widened.tool_runtime_config.browser.max_sessions = widened
        .tool_runtime_config
        .browser
        .max_sessions
        .saturating_add(1);

    let error = context
        .rebind_session(&widened)
        .expect_err("same-id Session must not replace its runtime authority");

    assert!(matches!(
        error,
        ContextSessionError::AuthorityExpanded {
            dimension: "tool runtime configuration"
        }
    ));
}

#[test]
fn recursive_session_rebind_accepts_same_session_runtime_narrowing() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let runtime_narrowing = crate::tools::runtime_config::ToolRuntimeNarrowing {
        browser: crate::tools::runtime_config::BrowserRuntimeNarrowing {
            max_sessions: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut narrowed = owner.session.clone();
    narrowed.tool_runtime_config = narrowed.tool_runtime_config.narrowed(&runtime_narrowing);
    narrowed.effective_runtime_narrowing = Some(runtime_narrowing);

    let rebound = context
        .rebind_session(&narrowed)
        .expect("same-id Session may narrow its runtime authority");

    assert_eq!(rebound.tool_runtime_config().browser.max_sessions, 1);
}

#[test]
fn recursive_session_rebind_rejects_runtime_narrowing_metadata_expansion() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let live_narrowing = crate::tools::runtime_config::ToolRuntimeNarrowing {
        web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
            allowed_domains: ["a.example".to_owned()].into_iter().collect(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut live = owner.session.clone();
    live.tool_runtime_config = live.tool_runtime_config.narrowed(&live_narrowing);
    live.effective_runtime_narrowing = Some(live_narrowing);
    let context = Context::new(&owner.runtime, &live).expect("live Session context");

    let expanded_narrowing = crate::tools::runtime_config::ToolRuntimeNarrowing {
        web_fetch: crate::tools::runtime_config::WebFetchRuntimeNarrowing {
            allowed_domains: ["a.example".to_owned(), "b.example".to_owned()]
                .into_iter()
                .collect(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut expanded = live.clone();
    expanded.effective_runtime_narrowing = Some(expanded_narrowing);

    let expanded_error = context
        .rebind_session(&expanded)
        .expect_err("metadata must not claim wider authority than the live Session");
    assert!(matches!(
        expanded_error,
        ContextSessionError::AuthorityExpanded {
            dimension: "tool runtime narrowing"
        }
    ));

    let mut removed = live.clone();
    removed.effective_runtime_narrowing = None;
    let removed_error = context
        .rebind_session(&removed)
        .expect_err("removing live narrowing metadata must not restore authority");
    assert!(matches!(
        removed_error,
        ContextSessionError::AuthorityExpanded {
            dimension: "tool runtime narrowing"
        }
    ));
}

#[test]
fn recursive_session_rebind_rejects_a_replaced_memory_backend() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let replacement = Session::root(
        owner.runtime.as_ref(),
        owner.session.agent_id(),
        owner.session.session_id(),
        owner.session.session_mode,
        owner.session.baseline_capabilities.clone(),
        owner.session.tool_runtime_config.clone(),
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        owner.session.tool_view.clone(),
        owner.session.workspace_root.clone(),
        None,
    )
    .expect("replacement backend fixture");
    let mut replaced = owner.session.clone();
    replaced.memory_backend = replacement.memory_backend;

    let error = context
        .rebind_session(&replaced)
        .expect_err("recursive execution must not switch memory backends");

    assert!(matches!(error, ContextSessionError::MemoryBackendChanged));
}

#[test]
fn detached_child_session_persists_the_parent_context_capability_ceiling() {
    let owner = crate::test_support::runtime_session_for_test(
        "parent-session",
        crate::tools::runtime_tool_view(),
    );
    let base = owner.context();
    let narrowed = base
        .derive_tool_child(Capabilities::from([Capability::MemoryRead]))
        .expect("narrow recursive context");
    let current = narrowed.allowed_capabilities();
    let child_tool_view = owner.session.tool_view.clone();
    let execution = crate::conversation::ConstrainedSubagentExecution {
        mode: crate::conversation::ConstrainedSubagentMode::Async,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: Some(crate::conversation::ConstrainedSubagentOwnerKind::AsyncDelegateSpawner),
        depth: 1,
        max_depth: 2,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: child_tool_view.tool_names().map(str::to_owned).collect(),
        capability_ceiling: current.as_ref().clone(),
        workspace_root: None,
        runtime_narrowing: Default::default(),
        identity: None,
        profile: None,
    };
    let child = owner
        .session
        .delegate_child("child-session", child_tool_view, execution, None)
        .expect("derive child Session");

    let detached = Context::new(&owner.runtime, &child)
        .expect("child Session must remain bound to the parent Runtime");

    assert_eq!(
        detached.allowed_capabilities().as_ref(),
        &Capabilities::from([Capability::MemoryRead])
    );
}

#[test]
fn context_rejects_a_session_from_another_runtime() {
    let config = LoongConfig::default();
    let owner_runtime = bootstrap_runtime_with_config(&config).expect("owner runtime");
    let other_runtime = bootstrap_runtime_with_config(&config).expect("unrelated runtime");
    let session = Session::from_config(
        owner_runtime.as_ref(),
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("owner session");

    let error = Context::new(other_runtime.as_ref(), &session)
        .expect_err("a Context cannot combine unrelated Runtime and Session owners");

    assert!(matches!(error, ContextSessionError::RuntimeMismatch));

    let owner_context = Context::new(owner_runtime.as_ref(), &session)
        .expect("owner Runtime must accept its own Session");
    let unrelated_session = Session::from_config(
        other_runtime.as_ref(),
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("unrelated session");
    let error = owner_context
        .rebind_session(&unrelated_session)
        .expect_err("recursive Context cannot rebind to another Runtime domain");

    assert!(matches!(error, ContextSessionError::RuntimeMismatch));
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn typed_tool_capability_override_rejects_added_capabilities() {
    let owner = crate::test_support::runtime_session_for_test(
        "test-session",
        crate::tools::runtime_tool_view(),
    );
    let context = owner.context();
    let invocation = context
        .tool(tool_path("read"))
        .expect("read should be registered");

    let error = invocation
        .with_capabilities_override(Capabilities::from([Capability::FilesystemWrite]))
        .invoke(serde_json::json!({ "path": "notes.txt" }))
        .await
        .expect_err("override must not add capabilities");

    assert!(
        error
            .to_string()
            .contains("not a subset of declared capabilities"),
        "expected capability override rejection, got: {error}"
    );
}

#[cfg(feature = "tool-file")]
#[tokio::test]
async fn typed_tool_capability_override_narrows_domain_action_caps() {
    let tempdir = tempdir().expect("tempdir");
    fs::write(tempdir.path().join("notes.txt"), "alpha").expect("write fixture");
    let mut config = LoongConfig::default();
    config.tools.file_root = Some(tempdir.path().display().to_string());
    let owner = crate::test_support::TestRuntimeSession::from_config(
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("runtime session");
    let context = owner.context();
    let invocation = context
        .tool(tool_path("read"))
        .expect("read should be registered");

    let error = invocation
        .with_capabilities_override(Capabilities::new())
        .invoke(serde_json::json!({ "path": "notes.txt" }))
        .await
        .expect_err("filesystem read should lose FilesystemRead capability");

    assert!(
        error.to_string().contains("FilesystemRead")
            || error.to_string().contains("filesystem_read"),
        "expected filesystem read capability denial, got: {error}"
    );
}
