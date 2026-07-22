use loong_contracts::{Capabilities, Capability, GovernedSessionMode};
use loong_core::policy::context::PolicyContext;
use loong_kernel::{
    access::memory::MemoryBackend,
    mailbox::{AgentPath, MailboxContent},
};

use super::{Session, absolute_lexical};
use crate::Context;
use crate::config::{AuditMode, LoongConfig};
use crate::conversation::InterAgentMessage;
use crate::runtime::bootstrap_runtime_with_config;
use crate::tools::ToolView;

impl Session {
    /// Replace only the concrete side-effect port in tests.
    ///
    /// Production selects this backend during Session materialization. Tests
    /// need the same ownership shape while observing or forcing typed backend
    /// outcomes, without registering a parallel Kernel adapter.
    pub(crate) fn with_memory_backend_for_test(
        mut self,
        backend: std::sync::Arc<
            dyn MemoryBackend<
                    StageEnvelope = crate::memory::StageEnvelope,
                    CompactOutput = crate::memory::StageDiagnostics,
                >,
        >,
    ) -> Self {
        self.memory_backend = backend;
        self
    }
}

#[test]
fn advisory_session_rejects_invocation_authority() {
    let runtime = bootstrap_runtime_with_config(&LoongConfig::default()).expect("test runtime");
    let result = Session::root(
        runtime.as_ref(),
        "advisory-agent",
        "advisory-session",
        GovernedSessionMode::AdvisoryOnly,
        Capabilities::from([Capability::InvokeTool, Capability::FilesystemRead]),
        crate::tools::runtime_config::ToolRuntimeConfig::default(),
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        crate::tools::runtime_tool_view(),
        None,
        None,
    );

    assert_eq!(
        result.expect_err("advisory authority must be internally consistent"),
        "advisory session cannot carry invocation or mutation authority"
    );
}

#[test]
fn root_session_rejects_empty_identity() {
    let runtime = bootstrap_runtime_with_config(&LoongConfig::default()).expect("test runtime");
    let result = Session::root(
        runtime.as_ref(),
        "test-agent",
        "  ",
        GovernedSessionMode::MutatingCapable,
        Capabilities::new(),
        crate::tools::runtime_config::ToolRuntimeConfig::default(),
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        ToolView::default(),
        None,
        None,
    );

    assert_eq!(
        result.expect_err("empty identity must fail"),
        "session id must not be empty"
    );
}

#[test]
fn session_debug_omits_executor_credentials() {
    let runtime = bootstrap_runtime_with_config(&LoongConfig::default()).expect("test runtime");
    let mut tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig::default();
    tool_runtime_config.web_search.brave_api_key = Some("secret-api-key".to_owned());
    let session = Session::root(
        runtime.as_ref(),
        "test-agent",
        "test-session",
        GovernedSessionMode::MutatingCapable,
        Capabilities::from([Capability::InvokeTool]),
        tool_runtime_config,
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        crate::tools::runtime_tool_view(),
        None,
        None,
    )
    .expect("session");

    let debug = format!("{session:?}");
    assert!(!debug.contains("secret-api-key"));
    assert!(!debug.contains("ToolRuntimeConfig"));
}

#[cfg(unix)]
#[test]
fn session_keeps_filesystem_authority_roots_lexical() {
    let base = crate::test_utils::unique_temp_dir("session-lexical-fs-root");
    let target = base.join("target");
    let link = base.join("configured-link");
    std::fs::create_dir_all(&target).expect("create symlink target");
    std::os::unix::fs::symlink(&target, &link).expect("create configured root symlink");
    let runtime = bootstrap_runtime_with_config(&LoongConfig::default()).expect("test runtime");
    let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
        file_root: Some(link.clone()),
        workspace_root: Some(link.clone()),
        ..Default::default()
    };

    let session = Session::root(
        runtime.as_ref(),
        "test-agent",
        "lexical-root-session",
        GovernedSessionMode::MutatingCapable,
        Capabilities::from([Capability::FilesystemRead]),
        tool_runtime_config,
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        crate::tools::runtime_tool_view(),
        None,
        None,
    )
    .expect("session construction should not inspect the configured root");

    assert_eq!(session.fs_resolution_root, link);
    assert_eq!(
        session.fs_allowed_roots.as_slice(),
        std::slice::from_ref(&link)
    );
    assert_eq!(session.fs_authority_ceiling_roots, [link]);
    assert_ne!(session.fs_allowed_roots, [target]);
    std::fs::remove_dir_all(base).ok();
}

#[test]
fn relative_root_materialization_is_independent_of_path_existence() {
    let base = crate::test_utils::unique_temp_dir("session-relative-fs-root");
    let relative_root = std::path::PathBuf::from("configured/../future-root");
    let before_creation = absolute_lexical(relative_root.clone(), &base);

    std::fs::create_dir_all(base.join("future-root")).expect("create configured root later");
    let after_creation = absolute_lexical(relative_root, &base);

    assert_eq!(before_creation, base.join("future-root"));
    assert_eq!(after_creation, before_creation);
    std::fs::remove_dir_all(base).ok();
}

#[test]
fn child_relative_workspace_uses_parent_resolution_root() {
    let runtime = bootstrap_runtime_with_config(&LoongConfig::default()).expect("test runtime");
    let authority_root = crate::test_utils::unique_temp_dir("session-parent-fs-authority");
    let parent_workspace = authority_root.join("parent-workspace");
    let tool_runtime_config = crate::tools::runtime_config::ToolRuntimeConfig {
        file_root: Some(authority_root),
        workspace_root: Some(parent_workspace.clone()),
        ..Default::default()
    };
    let capabilities = Capabilities::from([Capability::FilesystemRead]);
    let parent = Session::root(
        runtime.as_ref(),
        "test-agent",
        "parent-session",
        GovernedSessionMode::MutatingCapable,
        capabilities.clone(),
        tool_runtime_config,
        crate::memory::runtime_config::MemoryRuntimeConfig::default(),
        ToolView::default(),
        None,
        None,
    )
    .expect("parent session");
    let execution = crate::conversation::ConstrainedSubagentExecution {
        mode: crate::conversation::ConstrainedSubagentMode::Inline,
        isolation: crate::conversation::ConstrainedSubagentIsolation::Shared,
        owner_kind: None,
        depth: 1,
        max_depth: 2,
        active_children: 0,
        max_active_children: 1,
        timeout_seconds: 60,
        allow_shell_in_child: false,
        child_tool_allowlist: Vec::new(),
        capability_ceiling: capabilities,
        workspace_root: Some("child-workspace".into()),
        runtime_narrowing: Default::default(),
        identity: None,
        profile: None,
    };

    let child = parent
        .delegate_child("child-session", ToolView::default(), execution, None)
        .expect("child session");
    let expected = parent_workspace.join("child-workspace");

    assert_eq!(child.workspace_root.as_ref(), Some(&expected));
    assert_eq!(child.fs_resolution_root, expected);
    assert_eq!(child.fs_allowed_roots, [expected]);
}

#[test]
fn child_session_derives_parent_identity_from_the_parent_owner() {
    let owner = crate::test_support::child_runtime_session_for_test(
        "child-session",
        "parent-session",
        crate::tools::runtime_tool_view(),
    );
    let child = owner.session;

    assert_eq!(child.parent_session_id.as_deref(), Some("parent-session"));
}

#[tokio::test]
async fn cloned_session_shares_its_owned_mailbox() {
    let owner = crate::test_support::runtime_session_for_test(
        "shared-mailbox-session",
        crate::tools::runtime_tool_view(),
    );
    let cloned_session = owner.session.clone();

    cloned_session
        .mailbox()
        .sender()
        .send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::StatusNotification {
                reason: "clone-delivery".to_owned(),
            },
            trigger_turn: true,
        })
        .expect("send through cloned Session");

    let received = owner.session.mailbox().drain().await;
    assert_eq!(received.len(), 1);
}

#[tokio::test]
async fn independent_sessions_with_the_same_id_have_isolated_mailboxes() {
    let first = crate::test_support::runtime_session_for_test(
        "same-session-id",
        crate::tools::runtime_tool_view(),
    );
    let second = crate::test_support::runtime_session_for_test(
        "same-session-id",
        crate::tools::runtime_tool_view(),
    );

    first
        .session
        .mailbox()
        .sender()
        .send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::StatusNotification {
                reason: "runtime-local-delivery".to_owned(),
            },
            trigger_turn: true,
        })
        .expect("send through first Session");

    assert!(second.session.mailbox().drain().await.is_empty());
    assert_eq!(first.session.mailbox().drain().await.len(), 1);
}

#[tokio::test]
async fn dropping_the_last_session_owner_closes_its_mailbox() {
    let mut subscription = {
        let owner = crate::test_support::runtime_session_for_test(
            "dropped-mailbox-session",
            crate::tools::runtime_tool_view(),
        );
        owner.session.mailbox().subscribe()
    };

    assert!(subscription.changed().await.is_err());
}

#[test]
fn configured_session_grants_network_egress() {
    let mut config = LoongConfig::default();
    config.audit.mode = AuditMode::InMemory;

    let runtime = bootstrap_runtime_with_config(&config).expect("runtime bootstrap");
    let session = Session::from_config(
        runtime.as_ref(),
        &config,
        "test-session",
        "test-agent",
        GovernedSessionMode::MutatingCapable,
    )
    .expect("session");
    let context = Context::new(&runtime, &session)
        .expect("configured Session must remain bound to its Runtime");
    let allowed_capabilities = context.allowed_capabilities();

    assert!(
        allowed_capabilities.contains(Capability::InvokeTool),
        "session should retain invoke tool capability"
    );
    assert!(
        allowed_capabilities.contains(Capability::NetworkEgress),
        "session should grant network egress for governed web tools"
    );
}

#[test]
fn visible_skill_roots_bound_persisted_active_roots() {
    let root = crate::test_utils::unique_temp_dir("session-skill-root-ceiling");
    let visible_root = root.join("visible");
    let active_root = visible_root.join("active");
    let outside_root = root.join("outside");
    std::fs::create_dir_all(&active_root).expect("create active skill root");
    std::fs::create_dir_all(&outside_root).expect("create outside skill root");
    let expected_active_root = active_root.clone();
    let mut owner = crate::test_support::runtime_session_for_test(
        "skill-session",
        ToolView::from_legacy_paths(["read"]),
    );

    owner.session = owner
        .session
        .with_active_skill_roots(vec![active_root, outside_root])
        .with_visible_skill_roots(vec![visible_root]);

    assert_eq!(owner.session.active_skill_roots, vec![expected_active_root]);
}
